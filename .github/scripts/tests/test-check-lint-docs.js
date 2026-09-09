const assert = require('node:assert/strict');
const { execFileSync, spawnSync } = require('node:child_process');
const { mkdtempSync, readFileSync, writeFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const test = require('node:test');
const { validate } = require('../check-lint-docs.js');

const valid = '# Example\n\n**Severity**: `Info`\n**ID**: `example`\n\n'
    + '## What it does\n\nReports a pattern.\n\n## Why is this bad?\n\nExplains the consequence.\n\n'
    + '## Example\n\n```solidity\nbad();\n```\n\nUse instead:\n\n```solidity\ngood();\n```\n';

test('accepts both rationale headings, every severity, and CRLF', () => {
    for (const severity of ['High', 'Med', 'Low', 'Info', 'Gas', 'CodeSize']) {
        validate(valid.replace('`Info`', `\`${severity}\``), 'example');
    }
    validate(valid.replace('Why is this bad?', 'Why restrict this?'), 'example');
    validate(valid.replaceAll('\n', '\r\n'), 'example');
});

test('accepts the existing optional sections', () => {
    validate(valid.replace('## Why', '## Known limitations\n\nAn exclusion.\n\n## Why')
        + '\n## Configuration\n\n```toml\nsetting = true\n```\n\n## Notes\n\nA note.\n\n## Limitations\n\nA limitation.\n', 'example');
});

for (const [name, text, expected] of [
    ['missing title', valid.replace('# Example', 'Example'), 'start with one # title'],
    ['extra title', valid + '\n# Extra\n', 'only one # title'],
    ['missing severity', valid.replace('**Severity**: `Info`', ''), '**Severity**'],
    ['invalid severity', valid.replace('`Info`', '`Critical`'), '**Severity**'],
    ['wrong ID', valid.replace('`example`', '`other`'), 'matching the filename'],
    ['introductory summary', valid.replace('## What', 'Summary.\n\n## What'), 'no introductory summary'],
    ['duplicate section', valid + '\n## Example\n', 'duplicate section'],
    ['missing rationale', valid.replace('## Why is this bad?\n\nExplains the consequence.\n\n', ''), 'exactly one Why'],
    ['both rationales', valid + '\n## Why restrict this?\n\nPolicy.\n', 'exactly one Why'],
    ['wrong order', valid.replace('## Why is this bad?', '## Temporary').replace('## Example', '## Why is this bad?').replace('## Temporary', '## Example'), 'exactly one Why'],
    ['unknown section', valid + '\n## Scope and controls\n\nText.\n', 'unexpected section'],
    ['empty section', valid.replace('Reports a pattern.', ''), 'must not be empty'],
    ['empty optional section', valid + '\n## Notes\n\n### Detail\n', 'must not be empty'],
    ['no explanation', valid.replace('Reports a pattern.', '```solidity\nf();\n```'), 'explanatory prose'],
    ['repeated metadata', valid + '\n**ID**: `example`\n', 'metadata belongs only'],
    ['missing separator', valid.replace('Use instead:', ''), 'exactly one Use instead:'],
    ['duplicate separator', valid + '\nUse instead:\n', 'exactly one Use instead:'],
    ['Bad heading', valid.replace('Use instead:', '### Bad'), 'Bad/Good headings'],
    ['Good heading', valid.replace('Use instead:', '### Good'), 'Bad/Good headings'],
    ['empty triggering example', valid.replace('bad();', ''), 'nonempty solidity'],
    ['empty corrected example', valid.replace('good();', ' '), 'nonempty solidity'],
    ['wrong code language', valid.replace('```solidity', '```text'), 'nonempty solidity'],
    ['unclosed fence', valid.trimEnd().slice(0, -3), 'unclosed code fence'],
    ['mismatched fence', valid.replace('```solidity', '~~~solidity'), 'unclosed code fence'],
    ['short closing fence', valid.replace('```solidity', '````solidity'), 'unclosed code fence'],
]) {
    test(`rejects ${name}`, () => assert.throws(() => validate(text, 'example'), error => error.message.includes(expected)));
}

test('does not parse headings, metadata, or separators inside fenced code', () => {
    const source = valid.replace('bad();', '## Example\n**ID**: `other`\nUse instead:\n### Bad\n```');
    for (const fence of ['````', '~~~']) {
        validate(source.replaceAll('```solidity', `${fence}solidity`).replaceAll('\n```\n\n', `\n${fence}\n\n`).replace(/```\n$/, `${fence}\n`), 'example');
    }
});

test('rejects invalid filenames', () => {
    assert.throws(() => validate(valid, 'Not_kebab'), /filename must be a kebab-case lint ID/);
});

test('accepts the documented template after filling in metadata', () => {
    const template = readFileSync(join(__dirname, '../../../crates/lint/docs/_template.md'), 'utf8')
        .replace('`<High | Med | Low | Info | Gas | CodeSize>`', '`Info`')
        .replace('`<str_id>`', '`example`');
    validate(template, 'example');
});

test('checks the canonical directory independently of the working directory', () => {
    const script = join(__dirname, '../check-lint-docs.js');
    const output = execFileSync(process.execPath, [script], { cwd: tmpdir(), encoding: 'utf8' });
    assert.match(output, /Checked \d+ lint documentation files/);
});

test('CLI reports invalid files with a line number and nonzero exit status', () => {
    const root = mkdtempSync(join(tmpdir(), 'lint-doc-structure-'));
    try {
        const file = join(root, 'example.md');
        writeFileSync(file, valid.replace('Use instead:', ''));
        const result = spawnSync(process.execPath, [join(__dirname, '../check-lint-docs.js'), file], { encoding: 'utf8' });
        assert.equal(result.status, 1);
        assert.match(result.stderr, /example\.md:\d+: Example needs exactly one Use instead:/);
    } finally {
        rmSync(root, { recursive: true, force: true });
    }
});
