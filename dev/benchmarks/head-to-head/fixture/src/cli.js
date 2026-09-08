import { readFile } from 'node:fs/promises';
import { summarizeLedger } from './ledger.js';

const file = process.argv[2];
const text = await readFile(file, 'utf8');
process.stdout.write(JSON.stringify(summarizeLedger(text)) + '\n');
