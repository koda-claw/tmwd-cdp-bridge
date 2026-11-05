/**
 * Local HTTP client for `tmwd-cdp-bridge`.
 *
 * The client talks only to `127.0.0.1`, reads the bridge token file lazily on
 * each RPC call, and turns bridge error codes into actionable messages. It has
 * no dependency on DeepSeek Harness so it can be tested with plain Node.
 *
 * @module dsh-plugin-tmwd-cdp-bridge/bridge
 */

import { randomUUID } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'

/** Default HTTP RPC port used by the bridge. */
export const DEFAULT_HTTP_PORT = 18766
/** Default per-request timeout; the bridge itself also applies per-item timeouts. */
export const DEFAULT_REQUEST_TIMEOUT_MS = 120_000

/** Short recovery advice appended to well-known bridge error codes. */
const ERROR_HINTS = {
  UNAUTHORIZED:
    'The bridge rejected the token. Re-read the token file or restart the bridge, then retry.',
  NO_EXTENSION:
    'The bridge is running but the extension is not connected. Load or reload the tmwd-cdp-bridge unpacked extension and check the status tool again.',
  NO_SESSION:
    'No usable or matching tab. List tabs with tmwd_list_tabs and pass an existing session_id.',
  EXEC_TIMEOUT:
    'The page did not answer in time. Confirm the tab is responsive, or retry with a larger timeout.',
  EXEC_ERROR:
    'The page-side action failed. Inspect the message; for CSP or isolated-world failures retry with fallback: "cdp".',
  BAD_REQUEST:
    'The bridge rejected the request shape. Check the tool arguments and the RPC contract in docs/api.md.',
}

/**
 * A bridge or transport failure with a stable public code.
 */
export class BridgeError extends Error {
  /**
   * @param {string} code Stable error code (`UNAUTHORIZED`, `NO_EXTENSION`, ...).
   * @param {string} message Human-readable, token-free message.
   * @param {{ cause?: unknown }} [options]
   */
  constructor(code, message, options) {
    super(message, options)
    this.name = 'BridgeError'
    this.code = code
  }
}

/**
 * Append recovery advice once per message.
 *
 * @param {string} code Bridge error code.
 * @param {string} message Original bridge message.
 * @returns {string}
 */
function withHint(code, message) {
  const hint = ERROR_HINTS[code]
  if (hint === undefined || typeof message !== 'string') return message
  const trimmed = message.trim()
  if (trimmed.includes(hint)) return trimmed
  return trimmed === '' ? hint : `${trimmed} ${hint}`
}

/**
 * Validate and fill in plugin configuration.
 *
 * @param {Record<string, unknown>} [config]
 * @returns {{ httpPort: number, requestTimeoutMs: number, tokenFile?: string, appDir?: string }}
 */
export function normalizeConfig(config = {}) {
  const httpPort = config.httpPort ?? DEFAULT_HTTP_PORT
  if (!Number.isInteger(httpPort) || httpPort < 1 || httpPort > 65535) {
    throw new Error('httpPort must be an integer between 1 and 65535')
  }

  const requestTimeoutMs = config.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS
  if (!Number.isInteger(requestTimeoutMs) || requestTimeoutMs < 1000) {
    throw new Error('requestTimeoutMs must be an integer of at least 1000')
  }

  const result = { httpPort, requestTimeoutMs }
  for (const key of ['tokenFile', 'appDir']) {
    const value = config[key]
    if (value === undefined || value === null) continue
    if (typeof value !== 'string' || value.trim() === '') {
      throw new Error(`${key} must be a non-empty string when set`)
    }
    result[key] = value.trim()
  }
  return result
}

/**
 * Resolve the bridge application directory for the current platform.
 *
 * `CDP_BRIDGE_APP_DIR` matches the bridge's own override and wins over the
 * platform default. An explicit `appDir` in plugin configuration wins over both.
 *
 * @param {{ platform?: string, env?: Record<string, string | undefined>, home?: string }} [options]
 * @returns {string}
 */
export function defaultAppDir(options = {}) {
  const platform = options.platform ?? process.platform
  const env = options.env ?? process.env
  const home = options.home ?? homedir()

  const override = env.CDP_BRIDGE_APP_DIR
  if (typeof override === 'string' && override.trim() !== '') return override.trim()

  if (platform === 'darwin') {
    return join(home, 'Library', 'Application Support', 'tmwd-cdp-bridge')
  }
  if (platform === 'win32') {
    const localAppData = env.LOCALAPPDATA
    if (typeof localAppData === 'string' && localAppData.trim() !== '') {
      return join(localAppData, 'tmwd-cdp-bridge')
    }
    return join(home, 'AppData', 'Local', 'tmwd-cdp-bridge')
  }
  const xdgDataHome = env.XDG_DATA_HOME
  if (typeof xdgDataHome === 'string' && xdgDataHome.trim() !== '') {
    return join(xdgDataHome, 'tmwd-cdp-bridge')
  }
  return join(home, '.local', 'share', 'tmwd-cdp-bridge')
}

/**
 * Resolve the token file used by the bridge.
 *
 * @param {{ tokenFile?: string, appDir?: string }} settings
 * @param {{ platform?: string, env?: Record<string, string | undefined>, home?: string }} [options]
 * @returns {string}
 */
export function tokenPath(settings, options = {}) {
  if (settings.tokenFile !== undefined) return settings.tokenFile
  const appDir = settings.appDir ?? defaultAppDir(options)
  return join(appDir, 'token')
}

/**
 * Read and trim the bridge token.
 *
 * @param {string} path Absolute token path.
 * @returns {Promise<string>}
 * @throws {BridgeError} `TOKEN_MISSING` when the file is absent or empty.
 */
export async function readTokenFile(path) {
  let raw
  try {
    raw = await readFile(path, 'utf8')
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new BridgeError(
        'TOKEN_MISSING',
        `No bridge token at ${path}. Start the bridge with \`tmwd-cdp-bridge start\` so it creates the token file.`,
        { cause: error },
      )
    }
    throw new BridgeError('TOKEN_MISSING', `Cannot read the bridge token at ${path}.`, {
      cause: error,
    })
  }
  const token = raw.trim()
  if (token === '') {
    throw new BridgeError(
      'TOKEN_MISSING',
      `The bridge token at ${path} is empty. Start the bridge with \`tmwd-cdp-bridge start\` to regenerate it.`,
    )
  }
  return token
}

/**
 * Combine a caller signal with a request timeout.
 *
 * @param {AbortSignal | undefined} signal
 * @param {number} timeoutMs
 * @returns {AbortSignal}
 */
function timeoutSignal(signal, timeoutMs) {
  const timeout = AbortSignal.timeout(timeoutMs)
  return signal === undefined ? timeout : AbortSignal.any([signal, timeout])
}

/**
 * Create a client for one bridge configuration.
 *
 * @param {Record<string, unknown>} [config] Plugin configuration.
 * @param {{
 *   fetch?: typeof fetch,
 *   readToken?: (path: string) => Promise<string>,
 *   platform?: string,
 *   env?: Record<string, string | undefined>,
 *   home?: string,
 * }} [deps] Injectable dependencies used by tests.
 * @returns {{
 *   settings: { httpPort: number, requestTimeoutMs: number, tokenFile?: string, appDir?: string },
 *   httpUrl: string,
 *   tokenFile: string,
 *   health: (options?: { signal?: AbortSignal }) => Promise<Record<string, unknown>>,
 *   tokenPresent: () => Promise<boolean>,
 *   rpc: (command: Record<string, unknown>, options?: { signal?: AbortSignal, timeoutMs?: number }) => Promise<Record<string, unknown>>,
 * }}
 */
export function createBridgeClient(config = {}, deps = {}) {
  const settings = normalizeConfig(config)
  const fetchImpl = deps.fetch ?? globalThis.fetch
  const readToken = deps.readToken ?? readTokenFile
  const tokenFile = tokenPath(settings, deps)
  const httpUrl = `http://127.0.0.1:${settings.httpPort}`

  /**
   * @param {string} path
   * @param {{
   *   method?: string,
   *   body?: unknown,
   *   token?: string,
   *   timeoutMs?: number,
   *   signal?: AbortSignal,
   * }} [options]
   * @returns {Promise<Record<string, unknown>>}
   */
  async function send(path, options = {}) {
    const timeoutMs = options.timeoutMs ?? settings.requestTimeoutMs
    const headers = { accept: 'application/json' }
    if (options.token !== undefined) headers.authorization = `Bearer ${options.token}`
    let body
    if (options.body !== undefined) {
      headers['content-type'] = 'application/json'
      body = JSON.stringify(options.body)
    }

    let response
    try {
      response = await fetchImpl(`${httpUrl}${path}`, {
        method: options.method ?? 'GET',
        headers,
        body,
        signal: timeoutSignal(options.signal, timeoutMs),
      })
    } catch (error) {
      if (options.signal?.aborted === true) throw error
      if (error?.name === 'AbortError' || error?.name === 'TimeoutError') {
        throw new BridgeError(
          'BRIDGE_TIMEOUT',
          `tmwd-cdp-bridge did not answer within ${timeoutMs} ms at ${httpUrl}.`,
          { cause: error },
        )
      }
      throw new BridgeError(
        'BRIDGE_UNREACHABLE',
        `Cannot reach tmwd-cdp-bridge at ${httpUrl}. Start it with \`tmwd-cdp-bridge start\` and check \`tmwd-cdp-bridge status --json\`.`,
        { cause: error },
      )
    }

    let payload
    try {
      payload = await response.json()
    } catch (error) {
      throw new BridgeError(
        'BAD_RESPONSE',
        `tmwd-cdp-bridge returned a non-JSON response for ${path} (HTTP ${response.status}).`,
        { cause: error },
      )
    }

    const rpcError = payload?.r?.error
    if (!response.ok || rpcError !== undefined) {
      const code =
        response.status === 401 ? 'UNAUTHORIZED' : (rpcError?.code ?? `HTTP_${response.status}`)
      const message =
        rpcError?.message ??
        `tmwd-cdp-bridge returned HTTP ${response.status} for ${path}.`
      throw new BridgeError(code, withHint(code, message))
    }

    return payload
  }

  return {
    settings,
    httpUrl,
    tokenFile,

    /**
     * Read the unauthenticated health document.
     *
     * @param {{ signal?: AbortSignal }} [options]
     * @returns {Promise<Record<string, unknown>>}
     */
    async health(options = {}) {
      return send('/health', { signal: options.signal })
    },

    /**
     * Report whether a non-empty token file exists without exposing its value.
     *
     * @returns {Promise<boolean>}
     */
    async tokenPresent() {
      try {
        await readToken(tokenFile)
        return true
      } catch {
        return false
      }
    },

    /**
     * Call `POST /v1/rpc`.
     *
     * @param {Record<string, unknown>} command Bridge RPC command; a missing
     *   `request_id` is filled in.
     * @param {{ signal?: AbortSignal, timeoutMs?: number }} [options]
     * @returns {Promise<Record<string, unknown>>} The `r` envelope.
     */
    async rpc(command, options = {}) {
      const token = await readToken(tokenFile)
      const body = { request_id: randomUUID(), ...command }
      const response = await send('/v1/rpc', {
        method: 'POST',
        body,
        token,
        signal: options.signal,
        timeoutMs: options.timeoutMs,
      })
      return response.r ?? {}
    },
  }
}
