#!/usr/bin/env node
'use strict';

// `npx -y antirion [args]`: runs the Antirion binary of this package's version with the same
// arguments; with none, a gateway on localhost that is set up in its console.
require('../lib/launcher').main(process.argv.slice(2));
