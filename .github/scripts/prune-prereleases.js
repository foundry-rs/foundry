// In case node 21 is not used.
function groupBy(array, keyOrIterator) {
    var iterator;

    // use the function passed in, or create one
    if(typeof keyOrIterator !== 'function') {
        const key = String(keyOrIterator);
        iterator = function (item) { return item[key]; };
    } else {
        iterator = keyOrIterator;
    }

    return array.reduce(function (memo, item) {
        const key = iterator(item);
        memo[key] = memo[key] || [];
        memo[key].push(item);
        return memo;
    }, {});
}

module.exports = async ({ github, context }) => {
    console.log("Pruning old prereleases");

    // doc: https://docs.github.com/en/rest/releases/releases
    const { data: releases } = await github.rest.repos.listReleases({
        owner: context.repo.owner,
        repo: context.repo.repo,
    });

    let nightlies = releases.filter(
        release =>
            // Only consider releases tagged `nightly-${SHA}` for deletion
            release.tag_name.includes("nightly") &&
            release.tag_name !== "nightly"
    );

    // Pruning rules:
    //   1. only keep the earliest release of the month
    //   2. to keep the newest 3 nightlies
    // 
    // This addresses https://github.com/foundry-rs/foundry/issues/6732)

    // group releases by months
    const groups = groupBy(nightlies, i => i.created_at.slice(0, 7));
    const toPrune = Object.values(groups)
        .reduce((acc, cur) => acc.concat(cur.slice(0, -1)), [])
        .slice(3);

    for (const nightly of nightlies) {
        console.log(`Deleting nightly: ${nightly.tag_name}`);
        await github.rest.repos.deleteRelease({
            owner: context.repo.owner,
            repo: context.repo.repo,
            release_id: nightly.id,
        });
        console.log(`Deleting nightly tag: ${nightly.tag_name}`);
        await github.rest.git.deleteRef({
            owner: context.repo.owner,
            repo: context.repo.repo,
            ref: `tags/${nightly.tag_name}`,
        });
    }

    console.log("Done.");
};
