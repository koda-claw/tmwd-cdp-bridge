/**
 * DeepSeek Harness (Cordis) entry for the tmwd-cdp-bridge plugin.
 *
 * The bundle patch inserts this package as entry id `tmwd-cdp-bridge`. The
 * plugin registers native DSH tools that talk to the already-running local
 * bridge over `POST /v1/rpc`; it deliberately does not start, stop, or install
 * the bridge.
 *
 * @module dsh-plugin-tmwd-cdp-bridge
 */

import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'

import { createBridgeClient } from './bridge.js'
import { createToolDefinitions } from './tools.js'

/** Cordis plugin name. */
export const name = 'tmwd-cdp-bridge'

/** Services required before the tools can register. */
export const inject = ['tools']

/** Plugin configuration consumed from the profile patch entry. */
export const Config = z.object({
  httpPort: z.number().default(18766),
  requestTimeoutMs: z.number().default(120_000),
  tokenFile: z.string(),
  appDir: z.string(),
})

/**
 * Register the browser tools for this composition.
 *
 * @param {import('@deepseek-ai/cordis').Context} ctx
 * @param {Record<string, unknown>} [config]
 */
export function apply(ctx, config = {}) {
  const client = createBridgeClient(config)
  for (const definition of createToolDefinitions({ defineTool, client })) {
    ctx.tools.register(definition)
  }
}
