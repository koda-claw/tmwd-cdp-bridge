import assert from 'node:assert/strict'
import test from 'node:test'

import { BridgeError } from '../lib/bridge.js'
import { createToolDefinitions, renderJson } from '../lib/tools.js'

const identityDefineTool = (definition) => definition

/**
 * @param {Record<string, unknown>} [overrides]
 */
function makeClient(overrides = {}) {
  const calls = []
  return {
    httpUrl: 'http://127.0.0.1:18766',
    settings: { requestTimeoutMs: 120_000 },
    calls,
    health: async () => ({
      server: 'tmwd-cdp-bridge',
      version: '0.1.3',
      extension_connected: true,
      extension_id: 'eghifjkffmcmffejmaeeicejpfopplem',
    }),
    tokenPresent: async () => true,
    async rpc(command) {
      calls.push(command)
      return { request_id: 'mock', data: null }
    },
    ...overrides,
  }
}

/**
 * @param {ReturnType<typeof makeClient>} client
 */
function makeTools(client) {
  const definitions = createToolDefinitions({ defineTool: identityDefineTool, client })
  return Object.fromEntries(definitions.map((definition) => [definition.name, definition]))
}

test('registers the expected tool names', () => {
  const tools = makeTools(makeClient())
  assert.deepEqual(Object.keys(tools).sort(), [
    'tmwd_browser_status',
    'tmwd_cdp',
    'tmwd_execute_js',
    'tmwd_find_tab',
    'tmwd_list_tabs',
  ])
})

test('tmwd_browser_status reports a ready bridge', async () => {
  const tools = makeTools(makeClient())
  const value = await tools.tmwd_browser_status.execute({}, {})
  assert.equal(value.reachable, true)
  assert.equal(value.extension_connected, true)
  assert.equal(value.token_present, true)
  assert.match(value.hint, /ready/i)
  const rendered = tools.tmwd_browser_status.output.render({}, value)
  assert.match(rendered[0].text, /extension connected: yes/)
})

test('tmwd_browser_status degrades when the bridge is down', async () => {
  const tools = makeTools(
    makeClient({
      health: async () => {
        throw new BridgeError('BRIDGE_UNREACHABLE', 'start the bridge')
      },
      tokenPresent: async () => false,
    }),
  )
  const value = await tools.tmwd_browser_status.execute({}, {})
  assert.equal(value.reachable, false)
  assert.equal(value.token_present, false)
  assert.match(value.hint, /start the bridge/)
  const rendered = tools.tmwd_browser_status.output.render({}, value)
  assert.match(rendered[0].text, /not reachable/)
})

test('tmwd_browser_status warns when the extension is disconnected', async () => {
  const tools = makeTools(
    makeClient({
      health: async () => ({ server: 'tmwd-cdp-bridge', extension_connected: false }),
    }),
  )
  const value = await tools.tmwd_browser_status.execute({}, {})
  assert.equal(value.extension_connected, false)
  assert.match(value.hint, /extension is not connected/)
})

test('tmwd_list_tabs returns sanitized tab rows', async () => {
  const client = makeClient({
    async rpc(command) {
      client.calls.push(command)
      return {
        data: [
          {
            id: 7,
            url: 'https://example.com/',
            title: 'Example',
            active: true,
            window_id: 2,
            ignored: 'field',
          },
        ],
      }
    },
  })
  const tools = makeTools(client)
  const tabs = await tools.tmwd_list_tabs.execute({}, {})
  assert.deepEqual(client.calls[0], { cmd: 'get_all_sessions' })
  assert.deepEqual(tabs, [
    { id: 7, url: 'https://example.com/', title: 'Example', active: true, window_id: 2 },
  ])
  assert.match(tools.tmwd_list_tabs.output.render({}, tabs)[0].text, /^\* 7 {2}Example/)
})

test('tmwd_list_tabs renders an empty result', async () => {
  const tools = makeTools(makeClient())
  assert.equal(tools.tmwd_list_tabs.output.render({}, [])[0].text, '(no tabs)')
})

test('tmwd_find_tab requires a filter and forwards it', async () => {
  const client = makeClient({
    async rpc(command) {
      client.calls.push(command)
      return { data: { id: 3, url: 'https://example.com/', title: 'Example', active: false } }
    },
  })
  const tools = makeTools(client)
  await assert.rejects(tools.tmwd_find_tab.execute({}, {}), /url_contains or title_contains/)
  const value = await tools.tmwd_find_tab.execute({ url_contains: 'example.com' }, {})
  assert.deepEqual(client.calls[0], { cmd: 'find_session', url_contains: 'example.com' })
  assert.equal(value.found, true)
  assert.equal(value.tab.id, 3)
})

test('tmwd_find_tab reports no match', async () => {
  const tools = makeTools(makeClient({ rpc: async () => ({ data: null }) }))
  const value = await tools.tmwd_find_tab.execute({ title_contains: 'missing' }, {})
  assert.deepEqual(value, { found: false })
  assert.equal(tools.tmwd_find_tab.output.render({}, value)[0].text, 'No matching tab.')
})

test('tmwd_find_tab treats the bridge NO_SESSION error as no match', async () => {
  const tools = makeTools(
    makeClient({
      rpc: async () => {
        throw new BridgeError('NO_SESSION', 'no matching session')
      },
    }),
  )
  const value = await tools.tmwd_find_tab.execute({ title_contains: 'missing' }, {})
  assert.deepEqual(value, { found: false })
})

test('tmwd_execute_js forwards the bridge command and returns data', async () => {
  let timeoutMs
  const client = makeClient({
    async rpc(command, options) {
      client.calls.push(command)
      timeoutMs = options?.timeoutMs
      return { data: { title: 'Example' } }
    },
  })
  const tools = makeTools(client)
  const value = await tools.tmwd_execute_js.execute(
    {
      session_id: 42,
      code: 'document.title',
      timeout: 5,
      fallback: 'cdp',
    },
    {},
  )
  assert.deepEqual(client.calls[0], {
    cmd: 'execute_js',
    code: 'document.title',
    sessionId: '42',
    timeout: 5,
    fallback: 'cdp',
  })
  assert.equal(timeoutMs, 120_000)
  assert.deepEqual(value, { title: 'Example' })
})

test('tmwd_execute_js extends the HTTP timeout beyond the bridge timeout', async () => {
  let timeoutMs
  const client = makeClient({
    settings: { requestTimeoutMs: 10_000 },
    async rpc(_command, options) {
      timeoutMs = options?.timeoutMs
      return { data: null }
    },
  })
  const tools = makeTools(client)
  await tools.tmwd_execute_js.execute({ code: '1', timeout: 30 }, {})
  assert.equal(timeoutMs, 45_000)
})

test('tmwd_execute_js returns null when the bridge omits data', async () => {
  const tools = makeTools(makeClient({ rpc: async () => ({}) }))
  assert.equal(await tools.tmwd_execute_js.execute({ code: 'undefined' }, {}), null)
})

test('tmwd_cdp builds a direct CDP command', async () => {
  const client = makeClient({
    async rpc(command) {
      client.calls.push(command)
      return { data: { result: { type: 'string', value: 'Example' } } }
    },
  })
  const tools = makeTools(client)
  const value = await tools.tmwd_cdp.execute(
    {
      session_id: '9',
      method: 'Runtime.evaluate',
      params: { expression: 'document.title', returnByValue: true },
    },
    {},
  )
  assert.deepEqual(client.calls[0], {
    cmd: 'execute_js',
    mode: 'cdp',
    code: {
      method: 'Runtime.evaluate',
      params: { expression: 'document.title', returnByValue: true },
    },
    sessionId: '9',
  })
  assert.deepEqual(value, { result: { type: 'string', value: 'Example' } })
})

test('tmwd_cdp rejects non-object params', async () => {
  const tools = makeTools(makeClient())
  await assert.rejects(tools.tmwd_cdp.execute({ method: 'Runtime.evaluate', params: [] }, {}), /params/)
})

test('renderJson renders strings raw and objects as JSON', () => {
  assert.equal(renderJson('hello')[0].text, 'hello')
  assert.match(renderJson({ a: 1 })[0].text, /"a": 1/)
  assert.equal(renderJson(undefined)[0].text, 'undefined')
})
