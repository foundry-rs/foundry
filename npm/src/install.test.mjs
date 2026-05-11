import assert from 'node:assert/strict'
import { test } from 'node:test'

process.argv[2] = 'forge'

const { ensureSecureUrl } = await import('./install.mjs')

test('ensureSecureUrl rejects non-local HTTP URLs by default', () => {
  delete process.env.ALLOW_INSECURE_REGISTRY

  assert.throws(
    () => ensureSecureUrl('http://example.com/@foundry-rs/forge', 'registry URL'),
    /Refusing to use insecure HTTP for registry URL/
  )
})

test('ensureSecureUrl allows local HTTP registry URLs', () => {
  delete process.env.ALLOW_INSECURE_REGISTRY

  assert.doesNotThrow(() => {
    ensureSecureUrl('http://localhost:4873/@foundry-rs/forge', 'registry URL')
    ensureSecureUrl('http://127.0.0.1:4873/@foundry-rs/forge', 'registry URL')
    ensureSecureUrl('http://[::1]:4873/@foundry-rs/forge', 'registry URL')
  })
})

test('ensureSecureUrl allows explicit insecure registry override', () => {
  process.env.ALLOW_INSECURE_REGISTRY = 'true'

  assert.doesNotThrow(() => {
    ensureSecureUrl('http://example.com/@foundry-rs/forge', 'registry URL')
  })

  delete process.env.ALLOW_INSECURE_REGISTRY
})

test('ensureSecureUrl allows HTTPS URLs', () => {
  delete process.env.ALLOW_INSECURE_REGISTRY

  assert.doesNotThrow(() => {
    ensureSecureUrl('https://registry.npmjs.org/@foundry-rs/forge', 'registry URL')
  })
})

test('ensureSecureUrl leaves invalid URLs for the request layer', () => {
  delete process.env.ALLOW_INSECURE_REGISTRY

  assert.doesNotThrow(() => {
    ensureSecureUrl('not a url', 'registry URL')
  })
})
