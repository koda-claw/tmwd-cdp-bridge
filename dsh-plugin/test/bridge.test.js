import assert from 'node:assert/strict'
import http from 'node:http'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  BridgeError,
  createBridgeClient,
  defaultAppDir,
  normalizeConfig,
  readTokenFile,
  tokenPath,
} from '../lib/bridge.js'

/**
 * Start a local mock of the bridge HTTP API.
 *
 * @param {{ token?: string, delayMs?: number }} [options]
 * @returns {Promise<{ port: number, close: () => Promise<void> }>}
 */
async function startMockBridge(options = {}) {
  const token = options.token ?? 'secret'
  const delayMs = options.delayMs ?? 0
  const server = http.createServer((req, res) => {
    const chunks = []
    req.on('data', (chunk) => chunks.push(chunk))
    req.on('end', () => {
      const json = (status, body) => {
        res.writeHead(status, { 'content-type': 'application/json' })
        res.end(JSON.stringify(body))
      }
      if (req.method === 'GET' && req.url === '/health') {
        json(200, {
          server: 'tmwd-cdp-bridge',
          version: '0.1.3',
          extension_connected: true,
          extension_id: 'eghifjkffmcmffejmaaeicejpfopplem',
        })
        return
      }
      if (req.method === 'POST' && req.url === '/v1/rpc') {
        if (req.headers.authorization !== `Bearer ${token}`) {
          json(401, { r: { error: { code: 'UNAUTHORIZED', message: 'unauthorized' } } })
          return
        }
        const body = JSON.parse(Buffer.concat(chunks).toString('utf8'))
        setTimeout(() => {
          if (body.cmd === 'boom') {
            json(200, {
              r: {
                request_id: body.request_id,
                error: { code: 'NO_EXTENSION', message: 'not connected' },
              },
            })
            return
          }
          json(200, {
            r: {
              request_id: body.request_id,
              data: [{ id: 1, url: 'https://example.com/', title: 'Example', active: true }],
            },
          })
        }, delayMs)
        return
      }
      json(404, { error: 'not found' })
    })
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const address = server.address()
  if (address === null || typeof address === 'string') throw new Error('mock server has no port')
  return {
    port: address.port,
    async close() {
      server.closeAllConnections?.()
      await new Promise((resolve) => server.close(resolve))
    },
  }
}

/**
 * Create a temporary token file.
 *
 * @param {string} contents
 * @returns {Promise<{ dir: string, tokenFile: string }>}
 */
async function tempToken(contents) {
  const dir = await mkdtemp(join(tmpdir(), 'tmwd-dsh-plugin-'))
  const tokenFile = join(dir, 'token')
  await writeFile(tokenFile, contents)
  return { dir, tokenFile }
}

test('readTokenFile trims the token and reports missing or empty files', async () => {
  const { dir, tokenFile } = await tempToken('  secret-token\n')
  try {
    assert.equal(await readTokenFile(tokenFile), 'secret-token')
    await assert.rejects(readTokenFile(join(dir, 'missing')), (error) => {
      assert.ok(error instanceof BridgeError)
      assert.equal(error.code, 'TOKEN_MISSING')
      return true
    })
    await writeFile(tokenFile, '   \n')
    await assert.rejects(readTokenFile(tokenFile), (error) => {
      assert.ok(error instanceof BridgeError)
      assert.equal(error.code, 'TOKEN_MISSING')
      return true
    })
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('defaultAppDir follows the bridge platform rules', () => {
  assert.equal(
    defaultAppDir({ platform: 'darwin', home: '/Users/me', env: {} }),
    join('/Users/me', 'Library', 'Application Support', 'tmwd-cdp-bridge'),
  )
  assert.equal(
    defaultAppDir({ platform: 'linux', home: '/home/me', env: {} }),
    join('/home/me', '.local', 'share', 'tmwd-cdp-bridge'),
  )
  assert.equal(
    defaultAppDir({ platform: 'linux', home: '/home/me', env: { XDG_DATA_HOME: '/data' } }),
    join('/data', 'tmwd-cdp-bridge'),
  )
  assert.equal(
    defaultAppDir({ platform: 'win32', home: 'C:/Users/me', env: { LOCALAPPDATA: 'C:/Local' } }),
    join('C:/Local', 'tmwd-cdp-bridge'),
  )
  assert.equal(
    defaultAppDir({
      platform: 'darwin',
      home: '/Users/me',
      env: { CDP_BRIDGE_APP_DIR: '/custom/bridge' },
    }),
    '/custom/bridge',
  )
})

test('tokenPath prefers tokenFile, then appDir, then the platform default', () => {
  assert.equal(tokenPath({ tokenFile: '/custom/token' }), '/custom/token')
  assert.equal(tokenPath({ appDir: '/custom/app' }), join('/custom/app', 'token'))
  assert.equal(
    tokenPath({}, { platform: 'darwin', home: '/Users/me', env: {} }),
    join('/Users/me', 'Library', 'Application Support', 'tmwd-cdp-bridge', 'token'),
  )
})

test('normalizeConfig validates ports and timeouts', () => {
  assert.deepEqual(normalizeConfig({}), { httpPort: 18766, requestTimeoutMs: 120_000 })
  assert.throws(() => normalizeConfig({ httpPort: 0 }), /httpPort/)
  assert.throws(() => normalizeConfig({ httpPort: 1.5 }), /httpPort/)
  assert.throws(() => normalizeConfig({ requestTimeoutMs: 10 }), /requestTimeoutMs/)
  assert.throws(() => normalizeConfig({ tokenFile: '  ' }), /tokenFile/)
})

test('health reads the unauthenticated endpoint', async () => {
  const bridge = await startMockBridge()
  try {
    const client = createBridgeClient(
      { httpPort: bridge.port },
      {
        readToken: async () => {
          throw new Error('no token in this test')
        },
      },
    )
    const health = await client.health()
    assert.equal(health.server, 'tmwd-cdp-bridge')
    assert.equal(health.extension_connected, true)
    assert.equal(await client.tokenPresent(), false)
  } finally {
    await bridge.close()
  }
})

test('rpc sends the bearer token and returns the r envelope', async () => {
  const { dir, tokenFile } = await tempToken('secret\n')
  const bridge = await startMockBridge()
  try {
    const client = createBridgeClient({ httpPort: bridge.port, tokenFile })
    assert.equal(await client.tokenPresent(), true)
    const response = await client.rpc({ cmd: 'get_all_sessions' })
    assert.equal(typeof response.request_id, 'string')
    assert.deepEqual(response.data, [
      { id: 1, url: 'https://example.com/', title: 'Example', active: true },
    ])
  } finally {
    await bridge.close()
    await rm(dir, { recursive: true, force: true })
  }
})

test('rpc maps bridge error envelopes to actionable BridgeErrors', async () => {
  const { dir, tokenFile } = await tempToken('secret\n')
  const bridge = await startMockBridge()
  try {
    const client = createBridgeClient({ httpPort: bridge.port, tokenFile })
    await assert.rejects(client.rpc({ cmd: 'boom' }), (error) => {
      assert.ok(error instanceof BridgeError)
      assert.equal(error.code, 'NO_EXTENSION')
      assert.match(error.message, /load or reload/i)
      return true
    })
  } finally {
    await bridge.close()
    await rm(dir, { recursive: true, force: true })
  }
})

test('rpc reports an unauthorized token without leaking it', async () => {
  const { dir, tokenFile } = await tempToken('wrong-token\n')
  const bridge = await startMockBridge()
  try {
    const client = createBridgeClient({ httpPort: bridge.port, tokenFile })
    await assert.rejects(client.rpc({ cmd: 'get_all_sessions' }), (error) => {
      assert.ok(error instanceof BridgeError)
      assert.equal(error.code, 'UNAUTHORIZED')
      assert.doesNotMatch(error.message, /wrong-token/)
      return true
    })
  } finally {
    await bridge.close()
    await rm(dir, { recursive: true, force: true })
  }
})

test('rpc times out with BRIDGE_TIMEOUT', async () => {
  const { dir, tokenFile } = await tempToken('secret\n')
  const bridge = await startMockBridge({ delayMs: 1500 })
  try {
    const client = createBridgeClient({
      httpPort: bridge.port,
      tokenFile,
      requestTimeoutMs: 1000,
    })
    await assert.rejects(client.rpc({ cmd: 'get_all_sessions' }), (error) => {
      assert.ok(error instanceof BridgeError)
      assert.equal(error.code, 'BRIDGE_TIMEOUT')
      return true
    })
  } finally {
    await bridge.close()
    await rm(dir, { recursive: true, force: true })
  }
})

test('health reports BRIDGE_UNREACHABLE when nothing listens', async () => {
  const bridge = await startMockBridge()
  const port = bridge.port
  await bridge.close()
  const client = createBridgeClient({ httpPort: port, requestTimeoutMs: 1000 })
  await assert.rejects(client.health(), (error) => {
    assert.ok(error instanceof BridgeError)
    assert.equal(error.code, 'BRIDGE_UNREACHABLE')
    return true
  })
})
