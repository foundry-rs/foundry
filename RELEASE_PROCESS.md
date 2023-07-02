## Foundry Release Process

## Introduction

From 1.0 onwards, Foundry has a stable release process that is followed for each new version. This document serves as a guide and explanation for the process.

## Step 1 - Testing and benchmarking

Prior to creating a release commit and tagging it for release, we carefully test and benchmark the chosen commit. This is made to ensure that regressions are not made, and measure performance differences across versions.

## Step 2 - Create release commit

Creating a release commit involves the following steps:
- The `CHANGELOG.md` file is double-checked to be updated, and a new section indicating the new stable version is created, with the changes included.
- This `CHANGELO.md` is committed along with the tag name, e.g `v1.0.0`.

## Step 3 - Create tag and dispatch release workflow

- A tag is created for the new release commit.
- The tag is then pushed, and the release workflow will be automatically dispatched. The result of the workflow will be a new release with all the relevant files (binary, man pages and changelog).

## Step 4 - Release sanity test

- Once released, the new release is tested to ensure distribution channels (`foundryup`) can download and install the release.

## Miscellaneous

- The working branch is `master`. We do not follow a complicated `staging`/`master` separation, but rather choose to create tags at specific points on the branch history.
- The release channels are currently simple, and still based on `foundryup`. Different distribution channels might be considered down the line.