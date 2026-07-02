module.exports = async ({ github, context }, tagName) => {
    try {
        await github.rest.git.createRef({
            owner: context.repo.owner,
            repo: context.repo.repo,
            ref: `refs/tags/${tagName}`,
            sha: context.sha,
        });
    } catch (err) {
        if (err.status === 422) {
            const { data: existingRef } = await github.rest.git.getRef({
                owner: context.repo.owner,
                repo: context.repo.repo,
                ref: `tags/${tagName}`,
            });
            if (existingRef.object.sha === context.sha) {
                console.log(`Tag already exists at ${context.sha}: ${tagName}`);
                return;
            }
            console.error(`Tag ${tagName} already exists at ${existingRef.object.sha}, expected ${context.sha}`);
        }
        throw err;
    }
};
