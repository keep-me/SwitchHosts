/**
 * @author: oldj
 * @homepage: https://oldj.net
 *
 * Runtime dispatch between Electron `window._agent` and Tauri 2.
 * Electron path is unchanged. Tauri path routes through invoke/event.
 */

import { invoke } from '@tauri-apps/api/core'
import { emit, listen, once as tauriOnce, type UnlistenFn } from '@tauri-apps/api/event'
import { Actions } from '@common/types'

type AgentHandler = (...args: any[]) => void
type AgentOff = () => void

export interface IAgent {
  call: (action: string, ...params: any[]) => Promise<any>
  broadcast: (channel: string, ...args: any[]) => Promise<void> | void
  on: (channel: string, handler: AgentHandler) => AgentOff
  once: (channel: string, handler: AgentHandler) => AgentOff
  off: (channel: string, handler: AgentHandler) => void
  popupMenu: (options: any) => Promise<void> | void
  darkModeToggle: (theme: any) => Promise<void>
  platform: string
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

// ---- legacy name mapping ---------------------------------------------------

// Electron action names that v5 renames to a different Rust command.
// Unmapped names are converted to snake_case verbatim.
const LEGACY_TO_NEW: Record<string, string> = {
  getBasicData: 'get_basic_data',
  getList: 'get_list',
  getItemFromList: 'get_item_from_list',
  getContentOfList: 'get_content_of_list',
  getTrashcanList: 'get_trashcan_list',
  setList: 'set_list',
  moveToTrashcan: 'move_to_trashcan',
  moveManyToTrashcan: 'move_many_to_trashcan',
  clearTrashcan: 'clear_trashcan',
  deleteItemFromTrashcan: 'delete_item_from_trashcan',
  restoreItemFromTrashcan: 'restore_item_from_trashcan',

  getHostsContent: 'get_hosts_content',
  setHostsContent: 'set_hosts_content',
  getSystemHosts: 'get_system_hosts',
  getPathOfSystemHosts: 'get_path_of_system_hosts',
  // Semantic rename: Electron "setSystemHosts" took raw content; v5 takes
  // selection ids and aggregates inside hosts_apply.
  setSystemHosts: 'apply_hosts_selection',
  refreshHosts: 'refresh_remote_hosts',
  getHistoryList: 'get_apply_history',
  deleteHistory: 'delete_apply_history_item',

  cmdGetHistoryList: 'cmd_get_history_list',
  cmdDeleteHistory: 'cmd_delete_history_item',
  cmdClearHistory: 'cmd_clear_history',
  cmdFocusMainWindow: 'focus_main_window',

  findShow: 'find_show',
  findBy: 'find_by',
  findAddHistory: 'find_add_history',
  findGetHistory: 'find_get_history',
  findSetHistory: 'find_set_history',
  findAddReplaceHistory: 'find_add_replace_history',
  findGetReplaceHistory: 'find_get_replace_history',
  findSetReplaceHistory: 'find_set_replace_history',

  exportData: 'export_data',
  importData: 'import_data',
  importDataFromUrl: 'import_data_from_url',
  migrateCheck: 'migration_status',

  checkUpdate: 'check_update',
  downloadUpdate: 'download_update',
  installUpdate: 'install_update',

  openUrl: 'open_url',
  showItemInFolder: 'show_item_in_folder',
  updateTrayTitle: 'update_tray_title',
  closeMainWindow: 'hide_main_window',
  quit: 'quit_app',

  configGet: 'config_get',
  configSet: 'config_set',
  configAll: 'config_all',
  configUpdate: 'config_update',

  getDataDir: 'get_data_dir',
  ping: 'ping',
}

// Actions removed in v5. Calling them fails loudly so stray call sites don't
// silently no-op during migration.
const REMOVED = new Set<string>([
  'getDefaultDataDir',
  'cmdChangeDataDir',
  'cmdToggleDevTools',
  // migrateData is triggered internally by the Rust startup flow in v5 and is
  // no longer a renderer-invocable action.
  'migrateData',
])

function snakeCase(s: string): string {
  return s.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '')
}

function resolveCommandName(action: string): string {
  if (REMOVED.has(action)) {
    throw new Error(
      `[v5] action "${action}" was removed in the Tauri migration — see v5plan/switchhosts-v5-capabilities-and-commands.md`,
    )
  }
  return LEGACY_TO_NEW[action] ?? snakeCase(action)
}

// ---- tauri agent factory ---------------------------------------------------

function detectPlatform(): string {
  if (typeof navigator === 'undefined') return 'linux'
  const ua = navigator.userAgent.toLowerCase()
  if (ua.includes('mac')) return 'darwin'
  if (ua.includes('win')) return 'win32'
  return 'linux'
}

function makeTauriAgent(): IAgent {
  const listenerRegistry = new Map<string, Map<AgentHandler, Promise<UnlistenFn>>>()

  const on: IAgent['on'] = (channel, handler) => {
    const unlistenPromise = listen(channel, (event) => {
      const payload = event.payload as unknown
      if (Array.isArray(payload)) handler(...payload)
      else handler(payload)
    })
    let channelMap = listenerRegistry.get(channel)
    if (!channelMap) {
      channelMap = new Map()
      listenerRegistry.set(channel, channelMap)
    }
    channelMap.set(handler, unlistenPromise)
    return () => {
      unlistenPromise.then((un) => un()).catch(() => {})
      channelMap!.delete(handler)
    }
  }

  const off: IAgent['off'] = (channel, handler) => {
    const channelMap = listenerRegistry.get(channel)
    const unlistenPromise = channelMap?.get(handler)
    if (unlistenPromise) {
      unlistenPromise.then((un) => un()).catch(() => {})
      channelMap!.delete(handler)
    }
  }

  return {
    call: async (action, ...params) => {
      const cmd = resolveCommandName(action)
      return await invoke(cmd, { args: params })
    },

    broadcast: async (channel, ...args) => {
      await emit(channel, args.length <= 1 ? args[0] : args)
    },

    on,
    off,

    once: (channel, handler) => {
      const unlistenPromise = tauriOnce(channel, (event) => {
        const payload = event.payload as unknown
        if (Array.isArray(payload)) handler(...payload)
        else handler(payload)
      })
      return () => {
        unlistenPromise.then((un) => un()).catch(() => {})
      }
    },

    popupMenu: (options) => {
      // TODO Phase 1B: route through Rust tauri::menu API and emit
      // popup_menu_item_* / popup_menu_close:<menu_id> events back.
      console.warn('[v5] popupMenu not yet wired in Tauri adapter', options)
    },

    darkModeToggle: async (theme) => {
      await invoke('dark_mode_toggle', { args: [theme] })
    },

    platform: detectPlatform(),
  }
}

// ---- export ----------------------------------------------------------------

export const agent: IAgent = isTauri
  ? makeTauriAgent()
  : ((window as any)._agent as IAgent)

export const actions: Actions = new Proxy(
  {},
  {
    get(_obj, key: keyof Actions) {
      return (...params: any[]) => agent.call(String(key), ...params)
    },
  },
) as Actions
