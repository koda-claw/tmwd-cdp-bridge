/**
 * Model-facing tool definitions for the tmwd-cdp-bridge DSH plugin.
 *
 * Definitions are built through the host-supplied `defineTool` so the DSH tool
 * registry validates parameter and output schemas. The module itself has no
 * DeepSeek Harness import, which keeps the definitions unit-testable.
 *
 * @module tmwd-cdp-bridge-dsh-plugin/tools
 */

import { BridgeError } from './bridge.js'

/** Largest rendered tool result retained in the conversation. */
const MAX_RENDER_CHARS = 200_000

/** Default bridge-side execute_js timeout in seconds. */
const DEFAULT_EXEC_TIMEOUT_SECONDS = 15

/**
 * Truncate long page output while saying that it was cut.
 *
 * @param {string} text
 * @returns {string}
 */
function truncateText(text) {
  if (text.length <= MAX_RENDER_CHARS) return text
  return `${text.slice(0, MAX_RENDER_CHARS)}\n… [truncated ${text.length - MAX_RENDER_CHARS} characters]`
}

/**
 * Render any JSON value as tool text.
 *
 * @param {unknown} value
 * @returns {{ type: 'text', text: string }[]}
 */
export function renderJson(value) {
  let text
  if (typeof value === 'string') text = value
  else if (value === undefined) text = 'undefined'
  else text = JSON.stringify(value, null, 2)
  return [{ type: 'text', text: truncateText(text) }]
}

/**
 * @param {Record<string, unknown>} value
 * @returns {{ type: 'text', text: string }[]}
 */
function renderStatus(value) {
  if (value.reachable !== true) {
    const lines = [`tmwd-cdp-bridge is not reachable at ${value.http_url}.`]
    if (typeof value.hint === 'string' && value.hint !== '') lines.push('', value.hint)
    return [{ type: 'text', text: lines.join('\n') }]
  }
  const lines = [
    `tmwd-cdp-bridge${typeof value.version === 'string' ? ` ${value.version}` : ''} at ${value.http_url}`,
    `extension connected: ${value.extension_connected === true ? 'yes' : 'no'}`,
  ]
  if (typeof value.extension_id === 'string') lines.push(`extension id: ${value.extension_id}`)
  lines.push(`token file present: ${value.token_present === true ? 'yes' : 'no'}`)
  if (typeof value.hint === 'string' && value.hint !== '') lines.push('', value.hint)
  return [{ type: 'text', text: lines.join('\n') }]
}

/**
 * @param {unknown[]} tabs
 * @returns {{ type: 'text', text: string }[]}
 */
function renderTabs(tabs) {
  if (!Array.isArray(tabs) || tabs.length === 0) return [{ type: 'text', text: '(no tabs)' }]
  const lines = tabs.map((tab) => {
    const marker = tab.active === true ? '*' : '-'
    return `${marker} ${tab.id}  ${tab.title || '(untitled)'}  ${tab.url}`
  })
  return [{ type: 'text', text: lines.join('\n') }]
}

/**
 * @param {{ found: boolean, tab?: Record<string, unknown> }} value
 * @returns {{ type: 'text', text: string }[]}
 */
function renderFindTab(value) {
  if (value.found !== true || value.tab === undefined) {
    return [{ type: 'text', text: 'No matching tab.' }]
  }
  return [{ type: 'text', text: `${value.tab.id}  ${value.tab.title || '(untitled)'}  ${value.tab.url}` }]
}

/**
 * Sanitize one session row from the bridge.
 *
 * @param {unknown} raw
 * @returns {Record<string, unknown> | null}
 */
function toTab(raw) {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return null
  const tab = /** @type {Record<string, unknown>} */ (raw)
  const id = typeof tab.id === 'number' ? tab.id : Number(tab.id)
  if (!Number.isFinite(id)) return null
  return {
    id,
    url: typeof tab.url === 'string' ? tab.url : '',
    title: typeof tab.title === 'string' ? tab.title : '',
    active: tab.active === true,
    ...(typeof tab.window_id === 'number' ? { window_id: tab.window_id } : {}),
  }
}

/**
 * Tab JSON schema shared by tool outputs.
 *
 * @returns {Record<string, unknown>}
 */
function tabOutputSchema() {
  return {
    type: 'object',
    additionalProperties: false,
    properties: {
      id: { type: 'number', required: true, description: 'Bridge tab id.' },
      url: { type: 'string', required: true, description: 'Current tab URL.' },
      title: { type: 'string', required: true, description: 'Current tab title.' },
      active: {
        type: 'boolean',
        required: true,
        description: 'Whether the tab is active in its window.',
      },
      window_id: { type: 'number', description: 'Browser window id.' },
    },
  }
}

/**
 * Validate and convert a tool timeout in seconds to an HTTP timeout.
 *
 * @param {unknown} seconds
 * @param {number} floorMs
 * @returns {number}
 */
function requestTimeoutFor(seconds, floorMs) {
  if (seconds === undefined) return floorMs
  if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds <= 0) {
    throw new Error('timeout must be a positive number of seconds')
  }
  return Math.max(floorMs, Math.ceil(seconds * 1000) + 15_000)
}

/**
 * Build every tool definition this plugin registers.
 *
 * @param {{
 *   defineTool: (definition: Record<string, unknown>) => unknown,
 *   client: ReturnType<typeof import('./bridge.js').createBridgeClient>,
 * }} options
 * @returns {unknown[]}
 */
export function createToolDefinitions({ defineTool, client }) {
  return [
    defineTool({
      name: 'tmwd_browser_status',
      description:
        'Check whether the local tmwd-cdp-bridge is reachable and whether its Chrome/Edge extension is connected. Use this before browser work and after a browser call fails. It reads /health and checks that the token file exists; it does not run page code.',
      parameters: {},
      output: {
        schema: {
          type: 'object',
          additionalProperties: false,
          properties: {
            http_url: {
              type: 'string',
              required: true,
              description: 'Local bridge HTTP endpoint.',
            },
            reachable: {
              type: 'boolean',
              required: true,
              description: 'Whether /health answered.',
            },
            token_present: {
              type: 'boolean',
              required: true,
              description: 'Whether a non-empty token file exists.',
            },
            server: { type: 'string', description: 'Bridge server identity.' },
            version: { type: 'string', description: 'Bridge version.' },
            pid: { type: 'number', description: 'Bridge process id.' },
            extension_connected: {
              type: 'boolean',
              description: 'Whether the extension WebSocket is attached.',
            },
            extension_id: { type: 'string', description: 'Connected extension id.' },
            hint: { type: 'string', description: 'Next recovery or usage step.' },
          },
        },
        render: (_args, value) => renderStatus(value),
      },
      async execute(_args, exec) {
        let health
        try {
          health = await client.health({ signal: exec.signal })
        } catch (error) {
          if (
            error instanceof Error &&
            ['BRIDGE_UNREACHABLE', 'BRIDGE_TIMEOUT', 'BAD_RESPONSE'].includes(
              /** @type {{ code?: string }} */ (error).code ?? '',
            )
          ) {
            return {
              http_url: client.httpUrl,
              reachable: false,
              token_present: await client.tokenPresent(),
              hint: error.message,
            }
          }
          throw error
        }

        const tokenPresent = await client.tokenPresent()
        const connected = health.extension_connected === true
        let hint
        if (!connected) {
          hint =
            'The bridge is running but the extension is not connected. Load or reload the tmwd-cdp-bridge unpacked extension, then call this tool again.'
        } else if (!tokenPresent) {
          hint =
            'The bridge and extension are connected, but no token file was found. Start the bridge again so it writes the token file.'
        } else {
          hint = 'The bridge and extension are ready.'
        }

        return {
          http_url: client.httpUrl,
          reachable: true,
          token_present: tokenPresent,
          server: typeof health.server === 'string' ? health.server : 'tmwd-cdp-bridge',
          ...(typeof health.version === 'string' ? { version: health.version } : {}),
          ...(typeof health.pid === 'number' ? { pid: health.pid } : {}),
          extension_connected: connected,
          ...(typeof health.extension_id === 'string'
            ? { extension_id: health.extension_id }
            : {}),
          hint,
        }
      },
    }),

    defineTool({
      name: 'tmwd_list_tabs',
      description:
        'List every browser tab known to the connected tmwd-cdp-bridge extension, with its tab id, URL, title, and active state. Use this to choose a session_id before running JavaScript or CDP.',
      parameters: {},
      output: {
        schema: { type: 'array', items: tabOutputSchema() },
        render: (_args, tabs) => renderTabs(tabs),
      },
      async execute(_args, exec) {
        const response = await client.rpc({ cmd: 'get_all_sessions' }, { signal: exec.signal })
        const rows = Array.isArray(response.data) ? response.data : []
        return rows.map(toTab).filter((tab) => tab !== null)
      },
    }),

    defineTool({
      name: 'tmwd_find_tab',
      description:
        'Find the first browser tab whose URL or title contains the given text. Provide at least one filter. Use tmwd_list_tabs when no filter is known.',
      parameters: {
        url_contains: {
          type: 'string',
          description: 'Case-sensitive substring of the tab URL.',
        },
        title_contains: {
          type: 'string',
          description: 'Case-sensitive substring of the tab title.',
        },
      },
      output: {
        schema: {
          type: 'object',
          additionalProperties: false,
          properties: {
            found: { type: 'boolean', required: true, description: 'Whether a tab matched.' },
            tab: tabOutputSchema(),
          },
        },
        render: (_args, value) => renderFindTab(value),
      },
      async execute(args, exec) {
        if (args.url_contains === undefined && args.title_contains === undefined) {
          throw new Error('Provide url_contains or title_contains; use tmwd_list_tabs to inspect tabs.')
        }
        const command = {
          cmd: 'find_session',
          ...(args.url_contains === undefined ? {} : { url_contains: args.url_contains }),
          ...(args.title_contains === undefined ? {} : { title_contains: args.title_contains }),
        }
        let response
        try {
          response = await client.rpc(command, { signal: exec.signal })
        } catch (error) {
          // find_session reports no match as NO_SESSION; that is a normal miss
          // for this tool, not a failed browser operation.
          if (error instanceof BridgeError && error.code === 'NO_SESSION') {
            return { found: false }
          }
          throw error
        }
        const tab = toTab(response.data)
        return tab === null ? { found: false } : { found: true, tab }
      },
    }),

    defineTool({
      name: 'tmwd_execute_js',
      description:
        'Evaluate JavaScript in a tab of the local browser through tmwd-cdp-bridge and return its JSON-serializable result. Start with read-only inspection. Confirm with the user before destructive actions such as submitting forms, deleting data, changing settings, purchasing, or sending messages. If CSP or the page world blocks evaluation, retry with fallback: "cdp".',
      parameters: {
        session_id: {
          oneOf: [{ type: 'string' }, { type: 'number' }],
          description:
            'Tab id from tmwd_list_tabs. Omit to let the bridge pick the active scriptable tab, then the last successful tab, then the first scriptable tab.',
        },
        code: {
          type: 'string',
          required: true,
          description:
            'JavaScript source evaluated in the page. Return a JSON-serializable value, for example "document.title" or "({title: document.title, url: location.href})".',
        },
        timeout: {
          type: 'number',
          description: 'Bridge-side execution timeout in seconds. Defaults to 15.',
        },
        fallback: {
          type: 'string',
          enum: ['cdp'],
          description:
            'Retry with CDP when normal page-world evaluation fails because of CSP or isolated worlds.',
        },
      },
      output: {
        schema: { type: 'json' },
        render: (_args, value) => renderJson(value),
      },
      async execute(args, exec) {
        const command = {
          cmd: 'execute_js',
          code: args.code,
          ...(args.session_id === undefined ? {} : { sessionId: String(args.session_id) }),
          ...(args.timeout === undefined ? {} : { timeout: args.timeout }),
          ...(args.fallback === undefined ? {} : { fallback: args.fallback }),
        }
        const response = await client.rpc(command, {
          signal: exec.signal,
          timeoutMs: requestTimeoutFor(args.timeout, client.settings.requestTimeoutMs),
        })
        return response.data ?? null
      },
    }),

    defineTool({
      name: 'tmwd_cdp',
      description:
        'Send a raw Chrome DevTools Protocol command to a tab through tmwd-cdp-bridge. Use this for CDP-only methods or when CSP or isolated worlds block tmwd_execute_js. The extension needs its debugger permission. Confirm destructive actions with the user first.',
      parameters: {
        session_id: {
          oneOf: [{ type: 'string' }, { type: 'number' }],
          description: 'Tab id from tmwd_list_tabs. Omit to let the bridge choose a scriptable tab.',
        },
        method: {
          type: 'string',
          required: true,
          description: 'CDP method, for example "Runtime.evaluate".',
        },
        params: {
          type: 'json',
          description:
            'CDP params object, for example {"expression": "document.title", "returnByValue": true}.',
        },
        timeout: {
          type: 'number',
          description: 'Bridge-side execution timeout in seconds. Defaults to 15.',
        },
      },
      output: {
        schema: { type: 'json' },
        render: (_args, value) => renderJson(value),
      },
      async execute(args, exec) {
        if (args.params !== undefined && (args.params === null || typeof args.params !== 'object' || Array.isArray(args.params))) {
          throw new Error('params must be a CDP params object when set')
        }
        const command = {
          cmd: 'execute_js',
          mode: 'cdp',
          code: {
            method: args.method,
            params: args.params ?? {},
          },
          ...(args.session_id === undefined ? {} : { sessionId: String(args.session_id) }),
          ...(args.timeout === undefined ? {} : { timeout: args.timeout }),
        }
        const response = await client.rpc(command, {
          signal: exec.signal,
          timeoutMs: requestTimeoutFor(args.timeout, client.settings.requestTimeoutMs),
        })
        return response.data ?? null
      },
    }),
  ]
}
