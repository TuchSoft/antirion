'use strict';

// Runs the Antirion binary of this package's version. On the first run it downloads the release
// archive for this platform from GitHub, checks its SHA-256 against the release's checksums (the
// ones published with this package when it has them), and keeps the binary in a cache directory.

const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const https = require('node:https');
const os = require('node:os');
const path = require('node:path');
const tls = require('node:tls');
const zlib = require('node:zlib');
const { spawn } = require('node:child_process');

const RELEASES = 'https://github.com/eliasbiondo/antirion/releases/download';
const PACKAGE_DIR = path.join(__dirname, '..');
// The release binaries are built on Debian 12.
const MIN_GLIBC = [2, 36];
const SUPPORTED = 'Linux (glibc 2.36 or later) and macOS, on x64 or arm64';
const DOCKER = 'docker run -p 8080:8080 -p 8081:8081 eliasbiondo/antirion';

class LaunchError extends Error {}

/** The release's name for this platform, such as `darwin-arm64`; throws on any other platform. */
function target(platform, arch, glibc) {
    const archName = { x64: 'amd64', arm64: 'arm64' }[arch];
    const osName = { darwin: 'darwin', linux: 'linux' }[platform];
    if (!archName || !osName) {
        throw new LaunchError(
            `there is no Antirion binary for ${platform} on ${arch}: the releases cover ${SUPPORTED}. ` +
                `Run the container image instead (${DOCKER}), or build it from source.`,
        );
    }
    if (osName === 'linux') {
        if (!glibc) {
            throw new LaunchError(
                `this Linux has no glibc (musl, as on Alpine?): the Antirion binaries need glibc ` +
                    `${MIN_GLIBC.join('.')} or later. Run the container image instead: ${DOCKER}`,
            );
        }
        const [major, minor] = glibc.split('.').map(Number);
        if (major < MIN_GLIBC[0] || (major === MIN_GLIBC[0] && minor < MIN_GLIBC[1])) {
            throw new LaunchError(
                `the Antirion binaries need glibc ${MIN_GLIBC.join('.')} or later and this system has ${glibc}. ` +
                    `Run the container image instead: ${DOCKER}`,
            );
        }
    }
    return `${osName}-${archName}`;
}

/** The glibc version this Node runs on, or undefined (musl, or not Linux). */
function glibcVersion() {
    try {
        return process.report.getReport().header.glibcVersionRuntime;
    } catch {
        return undefined;
    }
}

function archiveName(version, platform) {
    return `antirion-v${version}-${platform}.tar.gz`;
}

function sumsName(version) {
    return `antirion-v${version}-sha256sums.txt`;
}

/** `sha256sum` output: a map from file name to its hex digest. */
function parseSums(text) {
    const sums = new Map();
    for (const line of text.split(/\r?\n/)) {
        const m = /^([0-9a-fA-F]{64}) [ *](.+)$/.exec(line.trim());
        if (m) sums.set(m[2].trim(), m[1].toLowerCase());
    }
    return sums;
}

/** The contents of the regular file `wanted` in a tar archive. */
function extractFile(tar, wanted) {
    const text = (buf, start, len) => {
        const field = buf.subarray(start, start + len);
        const end = field.indexOf(0);
        return field.subarray(0, end === -1 ? field.length : end).toString('utf8');
    };
    let offset = 0;
    let longName = null;
    while (offset + 512 <= tar.length) {
        const header = tar.subarray(offset, offset + 512);
        if (header.every((b) => b === 0)) break;
        const size = parseInt(text(header, 124, 12).trim() || '0', 8);
        if (!Number.isFinite(size) || size < 0) throw new LaunchError('the release archive is damaged');
        const type = String.fromCharCode(header[156] || 48);
        const prefix = text(header, 257, 6) === 'ustar' ? text(header, 345, 155) : '';
        const body = tar.subarray(offset + 512, offset + 512 + size);
        offset += 512 + Math.ceil(size / 512) * 512;
        if (type === 'x') {
            const pax = /(?:^|\n)\d+ path=([^\n]*)\n/.exec(body.toString('utf8'));
            longName = pax ? pax[1] : longName;
            continue;
        }
        if (type === 'L') {
            longName = text(body, 0, body.length);
            continue;
        }
        const name = (
            longName || (prefix ? `${prefix}/${text(header, 0, 100)}` : text(header, 0, 100))
        ).replace(/^\.\//, '');
        longName = null;
        if ((type === '0' || type === '7') && name === wanted) return Buffer.from(body);
    }
    throw new LaunchError(`the release archive has no ${wanted}`);
}

/** The proxy for `url` from the environment, honouring NO_PROXY; null for none. */
function proxyFor(url, env) {
    const u = new URL(url);
    const pick = (...names) => names.map((n) => env[n]).find((v) => v && v.trim());
    const proxy =
        u.protocol === 'https:'
            ? pick('https_proxy', 'HTTPS_PROXY', 'npm_config_https_proxy', 'npm_config_proxy')
            : pick('http_proxy', 'HTTP_PROXY', 'npm_config_proxy');
    if (!proxy) return null;
    const noProxy = (pick('no_proxy', 'NO_PROXY', 'npm_config_noproxy') || '')
        .split(/[,\s]+/)
        .filter(Boolean);
    const host = u.hostname.replace(/^\[|\]$/g, '').toLowerCase();
    const port = u.port || (u.protocol === 'https:' ? '443' : '80');
    for (const entry of noProxy) {
        if (entry === '*') return null;
        const [h, p] = entry
            .toLowerCase()
            .replace(/^\*?\./, '.')
            .split(/:(?=\d+$)/);
        if (p && p !== port) continue;
        const bare = h.replace(/^\./, '');
        if (host === bare || host.endsWith(`.${bare}`)) return null;
    }
    return new URL(proxy.includes('://') ? proxy : `http://${proxy}`);
}

function proxyAuthorization(proxy) {
    if (!proxy.username) return {};
    const user = `${decodeURIComponent(proxy.username)}:${decodeURIComponent(proxy.password)}`;
    return { 'proxy-authorization': `Basic ${Buffer.from(user).toString('base64')}` };
}

/** A socket tunnelled to `host:port` through an HTTP proxy with CONNECT. */
function tunnel(proxy, host, port) {
    return new Promise((resolve, reject) => {
        const authority = `${host.includes(':') ? `[${host}]` : host}:${port}`;
        const req = http.request({
            host: proxy.hostname.replace(/^\[|\]$/g, ''),
            port: proxy.port || 80,
            method: 'CONNECT',
            path: authority,
            headers: { host: authority, ...proxyAuthorization(proxy) },
        });
        req.once('connect', (res, socket) => {
            if (res.statusCode === 200) {
                resolve(socket);
            } else {
                socket.destroy();
                reject(
                    new LaunchError(
                        `the proxy ${proxy.host} refused the tunnel to ${authority}: HTTP ${res.statusCode}`,
                    ),
                );
            }
        });
        req.once('error', (e) =>
            reject(new LaunchError(`the proxy ${proxy.host} cannot be reached: ${e.message}`)),
        );
        req.end();
    });
}

/** GETs `url` into a buffer, following redirects, through the proxy the environment names. */
async function download(url, env, redirects = 5) {
    const u = new URL(url);
    const proxy = proxyFor(url, env);
    const headers = { 'user-agent': 'antirion-npm', accept: 'application/octet-stream' };
    let options;
    let client = u.protocol === 'https:' ? https : http;
    if (proxy && u.protocol === 'https:') {
        const socket = await tunnel(proxy, u.hostname.replace(/^\[|\]$/g, ''), u.port || 443);
        const secure = tls.connect({ socket, servername: u.hostname, ALPNProtocols: ['http/1.1'] });
        options = { method: 'GET', headers, createConnection: () => secure };
    } else if (proxy) {
        client = http;
        options = {
            host: proxy.hostname.replace(/^\[|\]$/g, ''),
            port: proxy.port || 80,
            path: u.href,
            method: 'GET',
            headers: { ...headers, host: u.host, ...proxyAuthorization(proxy) },
        };
    } else {
        options = { method: 'GET', headers };
    }
    const res = await new Promise((resolve, reject) => {
        const req = proxy && u.protocol !== 'https:' ? client.request(options) : client.request(u, options);
        req.once('response', resolve);
        req.once('error', (e) => reject(new LaunchError(`cannot download ${url}: ${e.message}`)));
        req.setTimeout(60_000, () => req.destroy(new Error('no answer for 60 s')));
        req.end();
    });
    if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location) {
        res.resume();
        if (redirects === 0) throw new LaunchError(`too many redirects downloading ${url}`);
        return download(new URL(res.headers.location, u).href, env, redirects - 1);
    }
    if (res.statusCode !== 200) {
        res.resume();
        throw new LaunchError(`cannot download ${url}: HTTP ${res.statusCode}`);
    }
    const chunks = [];
    for await (const chunk of res) chunks.push(chunk);
    return Buffer.concat(chunks);
}

function cacheRoot(env) {
    if (env.ANTIRION_CACHE_DIR) return path.resolve(env.ANTIRION_CACHE_DIR);
    const xdg = env.XDG_CACHE_HOME && path.isAbsolute(env.XDG_CACHE_HOME) ? env.XDG_CACHE_HOME : null;
    return path.join(xdg || path.join(os.homedir(), '.cache'), 'antirion');
}

/**
 * The path of the binary for `version`, downloaded and verified on the first call.
 * Options: version, env, platform, arch, glibc, packageDir (where checksums.txt may be), log.
 */
async function ensureBinary(opts) {
    const env = opts.env || process.env;
    const log = opts.log || ((line) => process.stderr.write(`${line}\n`));
    if (env.ANTIRION_BINARY) return env.ANTIRION_BINARY;
    const dir = path.join(cacheRoot(env), opts.version);
    const binary = path.join(dir, 'antirion');
    if (fs.existsSync(binary)) return binary;

    const platform = target(opts.platform || process.platform, opts.arch || process.arch, opts.glibc);
    const base = (env.ANTIRION_DOWNLOAD_URL || RELEASES).replace(/\/+$/, '');
    const release = `${base}/v${opts.version}`;
    const archive = archiveName(opts.version, platform);
    const bundled = path.join(opts.packageDir || PACKAGE_DIR, 'checksums.txt');
    const sumsText = fs.existsSync(bundled)
        ? fs.readFileSync(bundled, 'utf8')
        : (await download(`${release}/${sumsName(opts.version)}`, env)).toString('utf8');
    const expected = parseSums(sumsText).get(archive);
    if (!expected) throw new LaunchError(`the checksums of v${opts.version} do not list ${archive}`);

    log(`antirion: downloading Antirion v${opts.version} for ${platform} (once; it is cached in ${dir})`);
    const bytes = await download(`${release}/${archive}`, env);
    const actual = crypto.createHash('sha256').update(bytes).digest('hex');
    if (actual !== expected) {
        throw new LaunchError(
            `${archive} does not match its SHA-256 checksum (expected ${expected}, got ${actual})`,
        );
    }
    const executable = extractFile(zlib.gunzipSync(bytes), 'antirion');
    fs.mkdirSync(dir, { recursive: true });
    const partial = path.join(dir, `.antirion-${process.pid}-${crypto.randomBytes(4).toString('hex')}`);
    try {
        fs.writeFileSync(partial, executable, { mode: 0o755 });
        fs.chmodSync(partial, 0o755);
        fs.renameSync(partial, binary);
    } finally {
        fs.rmSync(partial, { force: true });
    }
    return binary;
}

/** Runs the binary with `args` and exits as it does. */
async function main(args) {
    const { version } = require(path.join(PACKAGE_DIR, 'package.json'));
    let binary;
    try {
        binary = await ensureBinary({
            version,
            glibc: process.platform === 'linux' ? glibcVersion() : undefined,
        });
    } catch (e) {
        process.stderr.write(`antirion: ${e instanceof LaunchError ? e.message : e.stack || e}\n`);
        process.exit(1);
    }
    const child = spawn(binary, args, { stdio: 'inherit' });
    // Ctrl-C reaches the gateway from the terminal; a second SIGINT would cut its drain short.
    process.on('SIGINT', () => {});
    for (const signal of ['SIGTERM', 'SIGHUP']) {
        process.on(signal, () => child.kill(signal));
    }
    child.once('error', (e) => {
        process.stderr.write(`antirion: cannot run ${binary}: ${e.message}\n`);
        process.exit(1);
    });
    child.once('exit', (code, signal) => {
        if (signal) {
            process.removeAllListeners(signal);
            process.kill(process.pid, signal);
        } else {
            process.exit(code ?? 1);
        }
    });
}

module.exports = {
    LaunchError,
    archiveName,
    download,
    ensureBinary,
    extractFile,
    main,
    parseSums,
    proxyFor,
    sumsName,
    target,
    tunnel,
};
