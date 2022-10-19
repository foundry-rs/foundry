module.exports = async ({ github, context }, tagName) => {
    try {
        await github.rest.git.updateRef({
            owner: context.repo.owner,
            repo: context.repo.repo,
            ref: `tags/${tagName}`,
            sha: context.sha,
            force: true,
        });
    } catch (err) {
        console.error(`Failed to move nightly tag.`);
        console.error(`This should only happen the first time.`);
        console.error(err);
    }
};
