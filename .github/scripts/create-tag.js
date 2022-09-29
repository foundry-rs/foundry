module.exports = async ({ github, context }, tagName) => {
    try {
        await github.rest.git.createRef({
            owner: context.repo.owner,
            repo: context.repo.repo,
            ref: `refs/tags/${tagName}`,
            sha: context.sha,
            force: true,
        });
    } catch (err) {
        console.error(`Failed to create tag: ${tagName}`);
        console.error(err);
    }
};
