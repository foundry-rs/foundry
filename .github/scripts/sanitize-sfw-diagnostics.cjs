// Temporary fetch/build Socket diagnostics. Never upload the raw directories.
const fs = require('node:fs');
const path = require('node:path');

const sensitive = /authorization|cookie|password|secret|token|api[_-]?key|private[_-]?key/i;

function redact(text, env = process.env) {
  for (const [key, value] of Object.entries(env)) {
    if (sensitive.test(key) && value.length >= 8) text = text.split(value).join('[REDACTED]');
  }
  return text
    .replace(/\x1b\[[0-9;]*m/g, '')
    .replace(/\b(?:Bearer|Basic)\s+[^\s"',}]+/gi, '[REDACTED AUTH]')
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '[REDACTED JWT]')
    .replace(/\bsktsec_[A-Za-z0-9_-]+/g, '[REDACTED SOCKET KEY]')
    .replace(/(https?:\/\/)[^\s/@]+:[^\s/@]+@/g, '$1[REDACTED]@')
    .replace(/([?&](?:[^=&#\s]*(?:token|key|secret|signature|credential)[^=&#\s]*)=)[^&#\s"']+/gi, '$1[REDACTED]')
    .split('\n').map(line => sensitive.test(line) && /[:=]/.test(line) ? '[REDACTED SENSITIVE LINE]' : line).join('\n');
}

function collect(phase = 'build') {
  if (!['fetch', 'build'].includes(phase)) throw new Error('Unknown diagnostic phase');
  const temporary = fs.realpathSync(process.env.RUNNER_TEMP);
  const raw = path.join(temporary, `sfw-mpp-${phase}-raw`);
  const output = path.join(temporary, 'sfw-mpp-diagnostics', phase);
  const logPath = path.join(raw, `${phase}.log`);
  if (!fs.existsSync(logPath)) {
    console.log(`No Socket ${phase} log: the ${phase} step did not start.`);
    return;
  }
  const log = fs.readFileSync(logPath, 'utf8');
  fs.mkdirSync(output, { recursive: true });
  const cleanLog = redact(log);
  fs.writeFileSync(path.join(output, `${phase}.log`), cleanLog);
  // Keep the error visible without interpreting log text as workflow commands.
  const marker = require('node:crypto').randomUUID();
  console.log(`::stop-commands::${marker}`);
  console.log(cleanLog.split('\n').slice(-100).join('\n'));
  console.log(`::${marker}::`);

  // Some Socket versions choose their own report path. Only accept a JSON file
  // explicitly identified by this phase, inside this job's temporary directory.
  const reports = new Set([path.join(raw, 'report.json')]);
  for (const match of log.matchAll(/sfw report written to:\s*([^\r\n\x1b]+\.json)/g)) reports.add(match[1].trim());
  let count = 0;
  for (const report of reports) {
    if (!fs.existsSync(report)) continue;
    const resolved = fs.realpathSync(report);
    if (!resolved.startsWith(temporary + path.sep) || !resolved.endsWith('.json')) continue;
    const data = JSON.parse(fs.readFileSync(resolved, 'utf8'));
    const clean = JSON.stringify(data, (key, value) => sensitive.test(key) ? '[REDACTED]' : typeof value === 'string' ? redact(value) : value, 2);
    fs.writeFileSync(path.join(output, `report-${++count}.json`), clean + '\n');
  }
  console.log(`Collected the ${phase} log and ${count} Socket JSON report(s).`);
}

if (require.main === module) collect(process.argv[2]);
module.exports = { redact, collect };
