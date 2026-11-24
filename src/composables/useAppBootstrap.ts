import { invoke } from "@tauri-apps/api/core";
import { check as checkForAppUpdate } from "@tauri-apps/plugin-updater";
import { useSettingStore } from "@/stores/setting";
import { useStatusStore } from "@/stores/status";
import { useDownloadStore } from "@/stores/download";
import {
  DEFAULT_OUTPUT_TEMPLATE,
  LEGACY_DEFAULT_OUTPUT_TEMPLATES,
} from "@/utils/output-template";
import type { ToolStatus, ToolUpdateCheck } from "@/types";

const TOOL_UPDATE_INTERVAL = 7 * 24 * 60 * 60 * 1000;
const TOOL_UPDATE_RETRY_INTERVAL = 2 * 24 * 60 * 60 * 1000;
const TOOL_UPDATE_POLL_INTERVAL = 60 * 60 * 1000;
let toolUpdateTimer: ReturnType<typeof setInterval> | undefined;
let toolUpdateInProgress = false;

/**
 * 封装应用启动引导与底层服务同步逻辑的 Composable：
 * 1. CLI 工具源设置同步（managed / system / custom）
 * 2. yt-dlp 分支通道同步（stable / nightly / master）
 * 3. 应用自动更新检查
 *
 * @returns 包含启动方法与各个同步操作的对象
 */
export const useAppBootstrap = () => {
  const settingStore = useSettingStore();
  const statusStore = useStatusStore();
  const downloadStore = useDownloadStore();

  const checkAndUpdateTools = async (): Promise<void> => {
    if (!settingStore.autoUpdateTools || toolUpdateInProgress) return;
    if (Date.now() < settingStore.nextToolUpdateAt) return;
    if (downloadStore.activeCount > 0) return;

    toolUpdateInProgress = true;
    let failed = false;
    try {
      await applyYtdlpChannel();
      for (const tool of ["yt-dlp", "deno", "ffmpeg"] as const) {
        try {
          const statusCommand = {
            "yt-dlp": "get_ytdlp_status",
            deno: "get_deno_status",
            ffmpeg: "get_ffmpeg_status",
          }[tool];
          const status = await invoke<ToolStatus>(statusCommand);
          if (!status.installed || !status.isManaged || !status.canUpdate) continue;

          const update = await invoke<ToolUpdateCheck>("check_tool_update", { tool });
          if (update.updateAvailable) {
            const updateCommand = {
              "yt-dlp": "update_ytdlp",
              deno: "update_deno",
              ffmpeg: "update_ffmpeg",
            }[tool];
            await invoke(updateCommand);
          }
        } catch {
          // One unavailable tool should not block the weekly check for the others.
          failed = true;
        }
      }
      settingStore.nextToolUpdateAt =
        Date.now() + (failed ? TOOL_UPDATE_RETRY_INTERVAL : TOOL_UPDATE_INTERVAL);
    } finally {
      toolUpdateInProgress = false;
    }
  };

  /**
   * 将前端持久化的 CLI 工具源同步至 Rust 后端
   *
   * @returns Promise<unknown>
   */
  const applyToolSources = (): Promise<unknown> =>
    invoke("set_tool_sources", {
      ytdlp: settingStore.ytdlpSource,
      deno: settingStore.denoSource,
      ffmpeg: settingStore.ffmpegSource,
      ytdlpLocalPath: settingStore.ytdlpLocalPath,
    });

  /**
   * 将前端配置的 yt-dlp 分支通道同步至 Rust 后端
   *
   * @returns Promise<unknown>
   */
  const applyYtdlpChannel = (): Promise<unknown> =>
    invoke("set_ytdlp_channel", { channel: settingStore.ytdlpChannel }).catch(() => {});

  /**
   * 检查应用是否有新版本可用，有更新时自动弹出模态框
   *
   * @returns Promise<void>
   */
  const checkAppUpdate = async (): Promise<void> => {
    try {
      const updateResult = await checkForAppUpdate();
      if (updateResult) {
        statusStore.updateVersion = updateResult.version;
        statusStore.updateNotes = updateResult.body || "";
        statusStore.showUpdateModal = true;
      }
    } catch {
      // 静默失败，不打扰用户正常使用
    }
  };

  /**
   * 执行完整的应用启动引导任务
   *
   * @returns Promise<void>
   */
  const bootstrap = async (): Promise<void> => {
    if (
      LEGACY_DEFAULT_OUTPUT_TEMPLATES.some((template) => template === settingStore.outputTemplate)
    ) {
      settingStore.outputTemplate = DEFAULT_OUTPUT_TEMPLATE;
    }

    watch(
      () => [
        settingStore.ytdlpSource,
        settingStore.ytdlpLocalPath,
        settingStore.denoSource,
        settingStore.ffmpegSource,
      ],
      () => applyToolSources(),
    );

    watch(
      () => settingStore.ytdlpChannel,
      () => applyYtdlpChannel(),
    );

    await applyToolSources();
    watch(
      () => settingStore.jsRuntime,
      (runtime) => invoke("set_js_runtime", { runtime }),
    );
    await invoke("set_js_runtime", { runtime: settingStore.jsRuntime });
    await applyYtdlpChannel();

    void checkAndUpdateTools();
    if (!toolUpdateTimer) {
      toolUpdateTimer = setInterval(() => void checkAndUpdateTools(), TOOL_UPDATE_POLL_INTERVAL);
    }

    if (settingStore.autoCheckUpdate) {
      void checkAppUpdate();
    }
  };

  return {
    bootstrap,
    applyToolSources,
    applyYtdlpChannel,
    checkAppUpdate,
  };
};
