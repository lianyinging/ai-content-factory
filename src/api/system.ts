import { invoke } from '@tauri-apps/api/core'
import { getName, getTauriVersion, getVersion } from '@tauri-apps/api/app'

export type SystemInfo = {
  appName: string
  version: string
  identifier: string
  tauriVersion: string
  os: string
  osFamily: string
  arch: string
  appDataDir: string
  resourceDir: string
}

export type UpdateCheckResult = {
  currentVersion: string
  latestVersion: string
  hasUpdate: boolean
  notes: string
  publishedAt: string
  downloadUrl: string
  pageUrl: string
  platformKey: string
  message: string
}

export const RELEASES_PAGE =
  'https://team-swarm-oss.oss-cn-shanghai.aliyuncs.com/ai_content/releases/'

const OS_LABEL: Record<string, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
  ios: 'iOS',
  android: 'Android',
}

export function formatOsLabel(os: string, arch: string) {
  const name = OS_LABEL[os] || os || '—'
  return arch ? `${name} · ${arch}` : name
}

export async function getSystemInfo(): Promise<SystemInfo> {
  const info = await invoke<SystemInfo>('get_system_info')
  // 前端 API 作兜底，保证展示名/版本与打包配置一致
  try {
    const [name, version, tauriVersion] = await Promise.all([
      getName(),
      getVersion(),
      getTauriVersion(),
    ])
    return {
      ...info,
      appName: name || info.appName,
      version: version || info.version,
      tauriVersion: tauriVersion || info.tauriVersion,
    }
  } catch {
    return info
  }
}

export async function checkAppUpdate(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>('check_app_update')
}
