module.exports = async ({ github, context }, tagName) => {
    try {
        await github.rest.git.createRef({
            owner: context.repo.owner,
            repo: context.repo.repo,
            ref: `refs/tags/${tagName}`,
            sha: context.sha,
        });
    } catch (err) {
        if (err.status === 422 && String(err.message).includes("Reference already exists")) {
            console.log(`Tag already exists: ${tagName}`);
            return;
        }
        throw err;
    }
};
