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
            release.tag_name !== "nightly" &&
            // ref: https://github.com/foundry-rs/foundry/issues/3881
            // Skipping pruning the build on 1st day of each month
            !release.created_at.includes("-01T")
    );

    // Keep newest 3 nightlies
    nightlies = nightlies.slice(3);

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
