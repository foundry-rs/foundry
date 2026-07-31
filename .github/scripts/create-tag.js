const resolveCommit = async ({ github, context }, object) => {
    const seen = new Set();
    while (object.type === 'tag') {
        if (seen.has(object.sha)) {
            throw new Error(`Tag object cycle detected at ${object.sha}`);
        }
        seen.add(object.sha);
        const { data: tag } = await github.rest.git.getTag({
            owner: context.repo.owner,
            repo: context.repo.repo,
            tag_sha: object.sha,
        });
        object = tag.object;
    }
    if (object.type !== 'commit') {
        throw new Error(`Tag resolves to ${object.type}, expected commit`);
    }
    return object.sha;
};

const existingTag = async ({ github, context }, tagName) => {
    try {
        const { data: ref } = await github.rest.git.getRef({
            owner: context.repo.owner,
            repo: context.repo.repo,
            ref: `tags/${tagName}`,
        });
        return ref.object;
    } catch (err) {
        if (err.status === 404) return null;
        throw err;
    }
};

const verifyExistingTag = async (client, tagName, targetSha, object) => {
    const existingSha = await resolveCommit(client, object);
    if (existingSha === targetSha) {
        console.log(`Tag already exists at ${targetSha}: ${tagName}`);
        return;
    }
    throw new Error(`Tag ${tagName} already resolves to ${existingSha}, expected ${targetSha}`);
};

module.exports = async (client, tagName, targetSha = client.context.sha) => {
    const object = await existingTag(client, tagName);
    if (object) {
        await verifyExistingTag(client, tagName, targetSha, object);
        return;
    }

    try {
        await client.github.rest.git.createRef({
            owner: client.context.repo.owner,
            repo: client.context.repo.repo,
            ref: `refs/tags/${tagName}`,
            sha: targetSha,
        });
    } catch (err) {
        if (err.status === 422) {
            const racedObject = await existingTag(client, tagName);
            if (racedObject) {
                await verifyExistingTag(client, tagName, targetSha, racedObject);
                return;
            }
        }
        throw err;
    }
};
