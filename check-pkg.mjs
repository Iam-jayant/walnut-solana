const fs = require('fs');
const p = JSON.parse(fs.readFileSync('/mnt/d/solana/walnut-solana/node_modules/@arcium-hq/client/package.json', 'utf8'));
console.log('dependencies:', JSON.stringify(p.dependencies, null, 2));
console.log('peerDependencies:', JSON.stringify(p.peerDependencies, null, 2));
console.log('exports:', JSON.stringify(Object.keys(p.exports || {}), null, 2));
