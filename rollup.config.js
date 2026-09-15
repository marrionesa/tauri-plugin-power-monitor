// Copyright 2019-2025 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { cwd } from 'node:process'

import { nodeResolve } from '@rollup/plugin-node-resolve'
import typescript from '@rollup/plugin-typescript'
import terser from '@rollup/plugin-terser'

const pkg = JSON.parse(readFileSync(join(cwd(), 'package.json'), 'utf8'))

// Derive the IIFE variable and the `window.__TAURI__` property name from the
// package name used by this plugin.
const pluginJsName = pkg.name
  .replace(/^(@[^/]+\/plugin-|@tauri-apps\/plugin-|tauri-plugin-)/, '')
  .replace(/-./g, (x) => x[1].toUpperCase())
const iifeVarName = `__TAURI_PLUGIN_${pkg.name
  .replace(/^(@[^/]+\/plugin-|@tauri-apps\/plugin-|tauri-plugin-)/, '')
  .replace(/-/g, '_')
  .toUpperCase()}__`

const external = [/^@tauri-apps\/api/]

export default [
  {
    input: 'guest-js/index.ts',
    output: [
      {
        file: pkg.exports.import,
        format: 'esm'
      },
      {
        file: pkg.exports.require,
        format: 'cjs'
      }
    ],
    plugins: [
      typescript({
        declaration: true,
        declarationDir: dirname(pkg.exports.import)
      })
    ],
    external: [
      ...external,
      ...Object.keys(pkg.dependencies || {}),
      ...Object.keys(pkg.peerDependencies || {})
    ],
    onwarn: (warning) => {
      throw Object.assign(new Error(), warning)
    }
  },

  {
    input: 'guest-js/index.ts',
    output: {
      format: 'iife',
      name: iifeVarName,
      // IIFE is in the format `var ${iifeVarName} = (() => {})()`
      // we check if __TAURI__ exists and inject the API object
      banner: "if ('__TAURI__' in window) {",
      // the last `}` closes the if in the banner
      footer: `Object.defineProperty(window.__TAURI__, '${pluginJsName}', { value: ${iifeVarName} }) }`,
      file: 'api-iife.js'
    },
    // and var is not guaranteed to assign to the global `window` object so we make sure to assign it
    plugins: [typescript(), terser(), nodeResolve()],
    onwarn: (warning) => {
      throw Object.assign(new Error(), warning)
    }
  }
]
