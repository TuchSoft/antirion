'use strict';

// The launcher against a fake release: a local HTTP server serves a temp directory laid out like
// the GitHub release downloads, with a shell script in place of the binary.

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const { execFile, execFileSync, spawnSync } = require('node:child_process');
const { after, before, describe, test } = require('node:test');

const launcher = require('../lib/launcher');

const VERSION = '1.2.3';
const PLATFORM = launcher.target(process.platform, process.arch, '2.36');
const ARCHIVE = launcher.archiveName(VERSION, PLATFORM);
const PACKAGE_VERSION = require('../package.json').version;

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'antirion-npm-test-'));
const releases = path.join(tmp, 'releases');
let server;
let base;
const requests = [];

/** A release `v<version>` whose `antirion` prints its arguments; returns the archive's SHA-256. */
function makeRelease(version) {
    const dir = path.join(releases, `v${version}`);
    const stage = path.join(tmp, `stage-${version}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.mkdirSync(stage, { recursive: true });
    fs.writeFileSync(path.join(stage, 'antirion'), '#!/bin/sh\necho "fake antirion $*"\nexit 7\n', {
        mode: 0o755,
    });
    const archive = launcher.archiveName(version, PLATFORM);
    execFileSync('tar', ['-C', stage, '-czf', path.join(dir, archive), 'antirion'], {
        env: { ...process.env, COPYFILE_DISABLE: '1' },
    });
    const sum = crypto
        .createHash('sha256')
        .update(fs.readFileSync(path.join(dir, archive)))
        .digest('hex');
    const other = launcher.archiveName(version, PLATFORM === 'linux-amd64' ? 'linux-arm64' : 'linux-amd64');
    fs.writeFileSync(
        path.join(dir, launcher.sumsName(version)),
        `${sum}  ${archive}\n${'0'.repeat(64)}  ${other}\n`,
    );
    return sum;
}

function cacheDir(name) {
    return path.join(tmp, 'cache', name);
}

function env(extra = {}) {
    return {
        ANTIRION_DOWNLOAD_URL: base,
        ANTIRION_CACHE_DIR: cacheDir('default'),
        NO_PROXY: '127.0.0.1',
        ...extra,
    };
}

const quiet = () => {};
// A package directory without checksums.txt: the checksums come from the fake release.
const noSums = path.join(tmp, 'pkg-empty');
fs.mkdirSync(noSums, { recursive: true });

/** Runs bin/antirion.js without blocking the fake release's server, which runs in this process. */
function runCli(args, extraEnv) {
    return new Promise((resolve) => {
        execFile(
            process.execPath,
            [path.join(__dirname, '..', 'bin', 'antirion.js'), ...args],
            { env: { ...process.env, ...extraEnv }, encoding: 'utf8' },
            (error, stdout, stderr) => resolve({ status: error ? error.code : 0, stdout, stderr }),
        );
    });
}

before(async () => {
    makeRelease(VERSION);
    makeRelease(PACKAGE_VERSION);
    server = http.createServer((req, res) => {
        requests.push(req.url);
        // Release downloads redirect, as GitHub's do.
        if (req.url.startsWith('/redirect/')) {
            res.writeHead(302, { location: req.url.slice('/redirect'.length) });
            res.end();
            return;
        }
        const file = path.join(releases, decodeURIComponent(req.url.split('?')[0]));
        if (!file.startsWith(releases) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
            res.writeHead(404);
            res.end('not found');
            return;
        }
        res.writeHead(200, { 'content-type': 'application/octet-stream' });
        fs.createReadStream(file).pipe(res);
    });
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    base = `http://127.0.0.1:${server.address().port}`;
});

after(() => {
    server.close();
    fs.rmSync(tmp, { recursive: true, force: true });
});

describe('platforms', () => {
    test('macOS and glibc Linux on x64 and arm64 have a binary', () => {
        assert.equal(launcher.target('darwin', 'arm64'), 'darwin-arm64');
        assert.equal(launcher.target('darwin', 'x64'), 'darwin-amd64');
        assert.equal(launcher.target('linux', 'x64', '2.36'), 'linux-amd64');
        assert.equal(launcher.target('linux', 'arm64', '2.39'), 'linux-arm64');
        assert.equal(launcher.target('linux', 'x64', '3.0'), 'linux-amd64');
    });

    test('anything else is refused with a reason and the container as the way out', () => {
        for (const [platform, arch, glibc, words] of [
            ['win32', 'x64', undefined, /no Antirion binary for win32/],
            ['linux', 'ia32', '2.36', /no Antirion binary for linux on ia32/],
            ['freebsd', 'x64', undefined, /no Antirion binary/],
            ['linux', 'x64', undefined, /no glibc/],
            ['linux', 'arm64', '2.35', /glibc 2.36 or later and this system has 2.35/],
        ]) {
            assert.throws(
                () => launcher.target(platform, arch, glibc),
                (e) =>
                    e instanceof launcher.LaunchError &&
                    words.test(e.message) &&
                    /docker run/.test(e.message),
            );
        }
    });
});

describe('archives and checksums', () => {
    test('sha256sum output parses, in text and binary mode', () => {
        const sums = launcher.parseSums(
            `${'a'.repeat(64)}  one.tar.gz\n${'B'.repeat(64)} *two.tar.gz\nnoise\n`,
        );
        assert.equal(sums.get('one.tar.gz'), 'a'.repeat(64));
        assert.equal(sums.get('two.tar.gz'), 'b'.repeat(64));
        assert.equal(sums.size, 2);
    });

    test('the binary comes out of a tar archive, with a long name header before it', () => {
        const block = (name, size, type) => {
            const h = Buffer.alloc(512);
            h.write(name, 0);
            h.write(`${size.toString(8).padStart(11, '0')}\0`, 124);
            h.write(type, 156);
            h.write('ustar\u000000', 257);
            return h;
        };
        const pad = (b) => Buffer.concat([b, Buffer.alloc((512 - (b.length % 512)) % 512)]);
        const pax = Buffer.from('16 path=antirion\n');
        const tar = Buffer.concat([
            block('other', 3, '0'),
            pad(Buffer.from('abc')),
            block('PaxHeader', pax.length, 'x'),
            pad(pax),
            block('ignored-name', 5, '0'),
            pad(Buffer.from('hello')),
            Buffer.alloc(1024),
        ]);
        assert.equal(launcher.extractFile(tar, 'antirion').toString(), 'hello');
        assert.throws(() => launcher.extractFile(tar, 'missing'), /no missing/);
    });
});

describe('download', () => {
    test('the first run downloads, verifies and caches; the next runs use the cache', async () => {
        const lines = [];
        const before = requests.length;
        const opts = {
            version: VERSION,
            env: env(),
            glibc: '2.36',
            packageDir: noSums,
            log: (l) => lines.push(l),
        };
        const binary = await launcher.ensureBinary(opts);
        assert.equal(binary, path.join(cacheDir('default'), VERSION, 'antirion'));
        assert.equal(fs.statSync(binary).mode & 0o111, 0o111);
        const run = spawnSync(binary, ['a', 'b'], { encoding: 'utf8' });
        assert.equal(run.status, 7);
        assert.equal(run.stdout, 'fake antirion a b\n');
        assert.deepEqual(requests.slice(before), [
            `/v${VERSION}/${launcher.sumsName(VERSION)}`,
            `/v${VERSION}/${ARCHIVE}`,
        ]);
        assert.match(lines.join('\n'), new RegExp(`downloading Antirion v${VERSION} for ${PLATFORM}`));
        assert.equal(await launcher.ensureBinary(opts), binary);
        assert.equal(requests.length, before + 2, 'the cached binary is used without a download');
        assert.deepEqual(
            fs.readdirSync(path.dirname(binary)),
            ['antirion'],
            'no partial file is left behind',
        );
    });

    test('redirects are followed', async () => {
        const binary = await launcher.ensureBinary({
            packageDir: noSums,
            version: VERSION,
            env: env({ ANTIRION_DOWNLOAD_URL: `${base}/redirect`, ANTIRION_CACHE_DIR: cacheDir('redirect') }),
            glibc: '2.36',
            log: quiet,
        });
        assert.ok(fs.existsSync(binary));
    });

    test('a checksum mismatch is refused and nothing is cached', async () => {
        const pkg = path.join(tmp, 'pkg-bad-sum');
        fs.mkdirSync(pkg, { recursive: true });
        fs.writeFileSync(path.join(pkg, 'checksums.txt'), `${'f'.repeat(64)}  ${ARCHIVE}\n`);
        const cache = cacheDir('bad-sum');
        await assert.rejects(
            launcher.ensureBinary({
                version: VERSION,
                env: env({ ANTIRION_CACHE_DIR: cache }),
                packageDir: pkg,
                glibc: '2.36',
                log: quiet,
            }),
            /does not match its SHA-256 checksum/,
        );
        assert.ok(!fs.existsSync(path.join(cache, VERSION, 'antirion')));
    });

    test('the checksums published with the package are used instead of the release file', async () => {
        const pkg = path.join(tmp, 'pkg-bundled');
        fs.mkdirSync(pkg, { recursive: true });
        fs.copyFileSync(
            path.join(releases, `v${VERSION}`, launcher.sumsName(VERSION)),
            path.join(pkg, 'checksums.txt'),
        );
        const before = requests.length;
        await launcher.ensureBinary({
            version: VERSION,
            env: env({ ANTIRION_CACHE_DIR: cacheDir('bundled') }),
            packageDir: pkg,
            glibc: '2.36',
            log: quiet,
        });
        assert.deepEqual(requests.slice(before), [`/v${VERSION}/${ARCHIVE}`]);
    });

    test('a version without a release, or without this platform, fails clearly', async () => {
        await assert.rejects(
            launcher.ensureBinary({
                packageDir: noSums,
                version: '9.9.9',
                env: env(),
                glibc: '2.36',
                log: quiet,
            }),
            /cannot download .*v9\.9\.9.*HTTP 404/,
        );
        const pkg = path.join(tmp, 'pkg-no-platform');
        fs.mkdirSync(pkg, { recursive: true });
        fs.writeFileSync(path.join(pkg, 'checksums.txt'), `${'0'.repeat(64)}  something-else.tar.gz\n`);
        await assert.rejects(
            launcher.ensureBinary({
                version: VERSION,
                env: env({ ANTIRION_CACHE_DIR: cacheDir('no-platform') }),
                packageDir: pkg,
                glibc: '2.36',
                log: quiet,
            }),
            new RegExp(`do not list ${ARCHIVE.replace(/\./g, '\\.')}`),
        );
    });

    test('ANTIRION_BINARY skips the download', async () => {
        assert.equal(
            await launcher.ensureBinary({ version: VERSION, env: { ANTIRION_BINARY: '/opt/antirion' } }),
            '/opt/antirion',
        );
    });
});

describe('proxies', () => {
    test('HTTPS_PROXY, HTTP_PROXY and NO_PROXY choose the proxy', () => {
        const e = {
            HTTPS_PROXY: 'http://proxy:3128',
            http_proxy: 'proxy2:8080',
            NO_PROXY: 'internal.example,.corp,10.0.0.1:8443',
        };
        assert.equal(launcher.proxyFor('https://github.com/x', e).host, 'proxy:3128');
        assert.equal(launcher.proxyFor('http://example.com/x', e).host, 'proxy2:8080');
        assert.equal(launcher.proxyFor('https://internal.example/x', e), null);
        assert.equal(launcher.proxyFor('https://a.b.corp/x', e), null);
        assert.equal(launcher.proxyFor('https://10.0.0.1:8443/x', e), null);
        assert.notEqual(launcher.proxyFor('https://10.0.0.1/x', e), null);
        assert.equal(launcher.proxyFor('https://github.com/x', { ...e, no_proxy: '*' }), null);
        assert.equal(launcher.proxyFor('https://github.com/x', {}), null);
    });

    test('a plain HTTP download goes through the proxy', async () => {
        const seen = [];
        const proxy = http.createServer((req, res) => {
            seen.push({ url: req.url, auth: req.headers['proxy-authorization'] });
            const upstream = http.request(req.url, { headers: req.headers }, (r) => {
                res.writeHead(r.statusCode, r.headers);
                r.pipe(res);
            });
            upstream.end();
        });
        await new Promise((resolve) => proxy.listen(0, '127.0.0.1', resolve));
        try {
            const proxyUrl = `http://user:p%40ss@127.0.0.1:${proxy.address().port}`;
            await launcher.ensureBinary({
                packageDir: noSums,
                version: VERSION,
                env: env({ HTTP_PROXY: proxyUrl, NO_PROXY: '', ANTIRION_CACHE_DIR: cacheDir('proxy') }),
                glibc: '2.36',
                log: quiet,
            });
            assert.equal(seen.length, 2);
            assert.equal(seen[1].url, `${base}/v${VERSION}/${ARCHIVE}`);
            assert.equal(seen[0].auth, `Basic ${Buffer.from('user:p@ss').toString('base64')}`);
        } finally {
            proxy.close();
        }
    });

    test('a CONNECT tunnel reaches the host through the proxy', async () => {
        const targets = [];
        const proxy = http.createServer();
        proxy.on('connect', (req, client, head) => {
            targets.push(req.url);
            const [host, port] = req.url.split(':');
            const upstream = net.connect(Number(port), host, () => {
                client.write('HTTP/1.1 200 Connection established\r\n\r\n');
                upstream.write(head);
                upstream.pipe(client);
                client.pipe(upstream);
            });
        });
        await new Promise((resolve) => proxy.listen(0, '127.0.0.1', resolve));
        try {
            const { port } = new URL(base);
            const socket = await launcher.tunnel(
                new URL(`http://127.0.0.1:${proxy.address().port}`),
                '127.0.0.1',
                port,
            );
            socket.write(
                `GET /v${VERSION}/${launcher.sumsName(VERSION)} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n`,
            );
            let answer = '';
            for await (const chunk of socket) answer += chunk;
            assert.match(answer, /^HTTP\/1\.1 200/);
            assert.match(answer, new RegExp(ARCHIVE.replace(/\./g, '\\.')));
            assert.deepEqual(targets, [`127.0.0.1:${port}`]);
        } finally {
            proxy.close();
        }
    });
});

describe('the antirion command', () => {
    test('runs the binary with the arguments and exits with its status', async () => {
        const run = await runCli(['serve', '--help'], env({ ANTIRION_CACHE_DIR: cacheDir('cli') }));
        assert.equal(run.status, 7, run.stderr);
        assert.equal(run.stdout, 'fake antirion serve --help\n');
        assert.match(
            run.stderr,
            new RegExp(`downloading Antirion v${PACKAGE_VERSION.replace(/\./g, '\\.')}`),
        );
    });

    test('a failure to download is one clear line', async () => {
        const run = await runCli(
            [],
            env({ ANTIRION_DOWNLOAD_URL: `${base}/nothing`, ANTIRION_CACHE_DIR: cacheDir('cli-fail') }),
        );
        assert.equal(run.status, 1);
        assert.match(run.stderr, /^antirion: cannot download .*HTTP 404\n$/);
    });
});
