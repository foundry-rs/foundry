module.exports = async ({github, context}) => {
  console.log('Pruning old prereleases')

  const { data: releases } = await github.rest.repos.listReleases({
    owner: context.repo.owner,
    repo: context.repo.repo
  })

  // Only consider releases tagged `nightly-${SHA}` for deletion
  let nightlies = releases.filter(
    (release) => release.tag_name.includes('nightly') && release.tag_name !== 'nightly'
  )

  // Keep newest 3 nightlies
  nightlies = nightlies.slice(3)

  for (const nightly of nightlies) {
    console.log(`Deleting nightly: ${nightly.tag_name}`)
    await github.rest.repos.deleteRelease({
      owner: context.repo.owner,
      repo: context.repo.repo,
      release_id: nightly.id
    })
    console.log(`Deleting nightly tag: ${nightly.tag_name}`)
    await github.rest.git.deleteRef({
      owner: context.repo.owner,
      repo: context.repo.repo,
      ref: `tags/${nightly.tag_name}`
    })
  }

  console.log('Done.')
}
