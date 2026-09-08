import { parseArguments } from './args.js';
import { loadDocument } from './load.js';
import { planBuild } from './index.js';
import { formatPlan } from './format.js';

const { file, options } = parseArguments(process.argv.slice(2));
process.stdout.write(formatPlan(planBuild(loadDocument(file), options)));
