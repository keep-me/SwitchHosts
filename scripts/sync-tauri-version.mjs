#!/usr/bin/env node
/**
 * Sync the `version` field in `src-tauri/tauri.conf.json` from the
 * single source of truth at `src/version.json`.
 *
 * Tauri's conf file only accepts three-segment semver, so we take
 * the first three elements of the `[major, minor, patch, build]`
 * array and join them with dots.
 *
 * Called automatically by `npm run build:renderer:tauri` (the
 * `beforeBuildCommand` in tauri.conf.json) and can also be run
 * manually: `node scripts/sync-tauri-version.mjs`
 */

import { readFileSync, writeFileSync } from 'fs'
import { join, dirname } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const root = join(__dirname, '..')

const version = JSON.parse(readFileSync(join(root, 'src/version.json'), 'utf-8'))
const semver = version.slice(0, 3).join('.')

const confPath = join(root, 'src-tauri/tauri.conf.json')
const conf = JSON.parse(readFileSync(confPath, 'utf-8'))

if (conf.version !== semver) {
  conf.version = semver
  writeFileSync(confPath, JSON.stringify(conf, null, 2) + '\n', 'utf-8')
  console.log(`[sync-tauri-version] updated tauri.conf.json version to ${semver}`)
} else {
  console.log(`[sync-tauri-version] tauri.conf.json version already ${semver}`)
}
