# -*- hcl -*-

/** Special target: https://github.com/docker/metadata-action#bake-definition */
// docker-bake.hcl
target "docker-metadata-action" {}

target "build" {
  inherits = ["docker-metadata-action"]
  context = "./"
  dockerfile = "Dockerfile"
  platforms = [
    "linux/amd64"
  ]
}
