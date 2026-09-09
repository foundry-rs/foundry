const { readFileSync, readdirSync } = require('node:fs');
const { basename, join } = require('node:path');

const docs = join(__dirname, '../../crates/lint/docs');
const severities = new Set(['High', 'Med', 'Low', 'Info', 'Gas', 'CodeSize']);
const required = ['What it does', 'Why is this bad?', 'Why restrict this?', 'Example'];
const optional = ['Configuration', 'Notes', 'Limitations', 'Known limitations'];
const fail = (line, message) => { throw new Error(`${line}: ${message}`); };

// Ignore Markdown-looking text inside fenced examples, including longer and tilde fences.
function tokens(text) {
    const result = [];
    let fence;
    for (const [index, line] of text.split(/\r?\n/).entries()) {
        const marker = line.match(/^ {0,3}(`{3,}|~{3,})(.*)$/);
        if (fence) {
            if (marker && marker[1][0] === fence.char && marker[1].length >= fence.length && !marker[2].trim()) {
                result.push({ kind: 'code', line: fence.line, language: fence.language, text: fence.lines.join('\n') });
                fence = undefined;
            } else fence.lines.push(line);
        } else if (marker) {
            fence = { char: marker[1][0], length: marker[1].length, language: marker[2].trim(), line: index + 1, lines: [] };
        } else if (line.trim()) {
            const heading = line.match(/^(#{1,6}) (.+)$/);
            result.push(heading
                ? { kind: 'heading', level: heading[1].length, text: heading[2].trim(), line: index + 1 }
                : { kind: 'text', text: line.trim(), line: index + 1 });
        }
    }
    if (fence) fail(fence.line, 'unclosed code fence');
    return result;
}

function validate(text, id) {
    if (!/^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/.test(id)) fail(1, 'filename must be a kebab-case lint ID');
    const items = tokens(text);
    const [title, severity, identity, firstSection] = items;
    if (title?.kind !== 'heading' || title.level !== 1 || !title.text)
        fail(1, 'start with one # title');
    const value = severity?.kind === 'text' && severity.text.match(/^\*\*Severity\*\*: `([^`]+)`$/)?.[1];
    if (!severities.has(value)) fail(severity?.line ?? 1, 'title must be followed by **Severity**: `<severity>`');
    if (identity?.kind !== 'text' || identity.text !== `**ID**: \`${id}\``)
        fail(identity?.line ?? 1, `severity must be followed by **ID**: \`${id}\` matching the filename`);
    if (firstSection?.kind !== 'heading' || firstSection.level !== 2 || firstSection.text !== 'What it does')
        fail(firstSection?.line ?? 1, 'start the body with ## What it does (no introductory summary)');

    const sections = [];
    for (const item of items.slice(3)) {
        if (item.kind === 'heading' && item.level <= 2) {
            if (item.level === 1) fail(item.line, 'only one # title is allowed');
            if (![...required, ...optional].includes(item.text)) fail(item.line, `unexpected section: ${item.text}`);
            if (sections.some(section => section.text === item.text)) fail(item.line, `duplicate section: ${item.text}`);
            sections.push({ ...item, content: [] });
        } else {
            if (item.kind === 'heading' && /^(Bad|Good)$/.test(item.text))
                fail(item.line, 'use Use instead: rather than Bad/Good headings');
            if (item.kind === 'text' && /^\*\*(Severity|ID)\*\*:/.test(item.text))
                fail(item.line, 'metadata belongs only below the title');
            sections.at(-1).content.push(item);
        }
    }
    const core = sections.filter(section => required.includes(section.text));
    if (core.length !== 3 || core[0].text !== 'What it does'
        || !['Why is this bad?', 'Why restrict this?'].includes(core[1].text) || core[2].text !== 'Example')
        fail(firstSection.line, 'expected What it does, exactly one Why section, then Example');
    for (const section of sections) {
        if (!section.content.some(item => item.kind !== 'heading' && item.text.trim()))
            fail(section.line, `${section.text} must not be empty`);
    }
    for (const section of core.slice(0, 2)) {
        if (!section.content.some(item => item.kind === 'text')) fail(section.line, `${section.text} must contain explanatory prose`);
    }
    const example = core[2];
    const separators = example.content.filter(item => item.kind === 'text' && item.text === 'Use instead:');
    if (separators.length !== 1) fail(example.line, 'Example needs exactly one Use instead: separator');
    const split = example.content.indexOf(separators[0]);
    for (const side of [example.content.slice(0, split), example.content.slice(split + 1)]) {
        if (!side.some(item => item.kind === 'code' && item.language === 'solidity' && item.text.trim()))
            fail(example.line, 'Example needs a nonempty solidity code block on each side of Use instead:');
    }
}

function main(args) {
    if (args.length === 1 && ['--help', '-h'].includes(args[0])) {
        console.log('Usage: node .github/scripts/check-lint-docs.js [file.md ...]\nWithout arguments, checks all lint docs except README.md and _template.md.');
        return;
    }
    const files = args.length ? args : readdirSync(docs).sort()
        .filter(file => file.endsWith('.md') && !['README.md', '_template.md'].includes(file))
        .map(file => join(docs, file));
    if (!files.length) throw new Error('no lint documentation found');
    let errors = 0;
    for (const file of files) {
        try {
            if (!file.endsWith('.md')) throw new Error('expected a Markdown file');
            validate(readFileSync(file, 'utf8'), basename(file, '.md'));
        } catch (error) {
            console.error(`${file}:${error.message}`);
            errors++;
        }
    }
    if (errors) process.exitCode = 1;
    else console.log(`Checked ${files.length} lint documentation files.`);
}

module.exports = { validate };
if (require.main === module) main(process.argv.slice(2));
