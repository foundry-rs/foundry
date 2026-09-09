const assert = require('node:assert/strict');
const test = require('node:test');

const createTag = require('../create-tag.js');

const notFound = () => Object.assign(new Error('Not Found'), { status: 404 });
const unprocessable = () => Object.assign(new Error('Unprocessable Entity'), { status: 422 });

const client = ({ getRef, getTag = async () => assert.fail('unexpected getTag'), createRef }) => ({
    context: { repo: { owner: 'foundry-rs', repo: 'foundry' }, sha: 'context-sha' },
    github: { rest: { git: { getRef, getTag, createRef } } },
});

test('creates a tag at the explicit commit', async () => {
    let request;
    const api = client({
        getRef: async () => { throw notFound(); },
        createRef: async (value) => { request = value; },
    });

    await createTag(api, 'v1.7.2', 'merged-sha');

    assert.deepEqual(request, {
        owner: 'foundry-rs',
        repo: 'foundry',
        ref: 'refs/tags/v1.7.2',
        sha: 'merged-sha',
    });
});

test('accepts an existing lightweight tag at the commit', async () => {
    const api = client({
        getRef: async () => ({ data: { object: { type: 'commit', sha: 'merged-sha' } } }),
        createRef: async () => assert.fail('unexpected createRef'),
    });

    await createTag(api, 'v1.7.2', 'merged-sha');
});

test('peels an existing annotated tag to its commit', async () => {
    const api = client({
        getRef: async () => ({ data: { object: { type: 'tag', sha: 'tag-object' } } }),
        getTag: async (request) => {
            assert.equal(request.tag_sha, 'tag-object');
            return { data: { object: { type: 'commit', sha: 'merged-sha' } } };
        },
        createRef: async () => assert.fail('unexpected createRef'),
    });

    await createTag(api, 'v1.7.2', 'merged-sha');
});

test('peels nested annotated tags to their terminal commit', async () => {
    const api = client({
        getRef: async () => ({ data: { object: { type: 'tag', sha: 'outer-tag' } } }),
        getTag: async ({ tag_sha: tagSha }) => ({
            data: {
                object: tagSha === 'outer-tag'
                    ? { type: 'tag', sha: 'inner-tag' }
                    : { type: 'commit', sha: 'merged-sha' },
            },
        }),
        createRef: async () => assert.fail('unexpected createRef'),
    });

    await createTag(api, 'v1.7.2', 'merged-sha');
});

test('rejects an existing tag at another commit without moving it', async () => {
    const api = client({
        getRef: async () => ({ data: { object: { type: 'commit', sha: 'other-sha' } } }),
        createRef: async () => assert.fail('unexpected createRef'),
    });

    await assert.rejects(
        createTag(api, 'v1.7.2', 'merged-sha'),
        /already resolves to other-sha, expected merged-sha/,
    );
});

test('accepts a same-commit create race', async () => {
    let reads = 0;
    const api = client({
        getRef: async () => {
            if (reads++ === 0) throw notFound();
            return { data: { object: { type: 'commit', sha: 'merged-sha' } } };
        },
        createRef: async () => { throw unprocessable(); },
    });

    await createTag(api, 'v1.7.2', 'merged-sha');
});

test('rejects a conflicting create race without moving the tag', async () => {
    let reads = 0;
    const api = client({
        getRef: async () => {
            if (reads++ === 0) throw notFound();
            return { data: { object: { type: 'commit', sha: 'other-sha' } } };
        },
        createRef: async () => { throw unprocessable(); },
    });

    await assert.rejects(
        createTag(api, 'v1.7.2', 'merged-sha'),
        /already resolves to other-sha, expected merged-sha/,
    );
});

test('preserves the context SHA default used by nightlies', async () => {
    let request;
    const api = client({
        getRef: async () => { throw notFound(); },
        createRef: async (value) => { request = value; },
    });

    await createTag(api, 'nightly-context-sha');

    assert.equal(request.sha, 'context-sha');
});
