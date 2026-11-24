<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { downloadDir, join } from "@tauri-apps/api/path";
import { open, save } from "@tauri-apps/plugin-dialog";
import { showErrorDialog, sanitizeFilename } from "@/utils/format";
import { isValidUrl } from "@/utils/validate";
import { useSettingStore } from "@/stores/setting";
import { useStatusStore } from "@/stores/status";
import { useVideoStore } from "@/stores/video";
import { goToolList } from "@/utils/toolbox";
import { useToolTask } from "@/composables/useToolTask";
import { composeOutputTemplate, renderOutputTemplate } from "@/utils/output-template";
import { useI18n } from "vue-i18n";
import type { ThumbnailBatchResult, ThumbnailInfo, ThumbnailVideoResult } from "@/types";

const { t } = useI18n();
const router = useRouter();
const settingStore = useSettingStore();
const statusStore = useStatusStore();
const videoStore = useVideoStore();

const goBack = () => goToolList(router);

const url = ref("");
const {
  state: taskState,
  result: taskResult,
  resultMeta: taskResultMeta,
  running: loading,
  start,
  cancel,
} = useToolTask<ThumbnailBatchResult>("thumbnail");
const thumbnailVideos = ref<ThumbnailVideoResult[]>([]);
const thumbnailErrors = ref<ThumbnailBatchResult["errors"]>([]);
const savingId = ref<string | null>(null);
const downloadHighestQuality = ref(false);
const autoSaveRunId = ref<string | null>(null);
const autoSavedRunId = ref<string | null>(null);
const lastFetchUsedHighestQuality = ref(false);
const highestQualitySaving = ref(false);
const highestQualitySaved = ref(0);
const highestQualitySaveDir = ref("");
const highestQualityVideoTotal = ref(0);
const currentFetchUrls = ref<string[]>([]);

const inputUrls = computed(() =>
  url.value
    .split(/[\r\n]+/)
    .map((item) => item.trim())
    .filter(Boolean),
);
const urlValid = computed(
  () => inputUrls.value.length > 0 && inputUrls.value.every((item) => isValidUrl(item)),
);
const thumbnailVideoCount = computed(() => thumbnailVideos.value.length);

/** 保存時にPNGへ変換する */
const getExtFromUrl = (_url: string): string => "png";

const getThumbnailFilename = (video: ThumbnailVideoResult): string => {
  const template = composeOutputTemplate(
    settingStore.outputTemplate,
    settingStore.filenamePrefix,
    settingStore.filenameSuffix,
  );
  const title = video.title || t("toolbox.thumbnailTitle");
  const rendered = renderOutputTemplate(template, {
    title,
    id: video.id,
    uploader: video.uploader,
    upload_date: video.uploadDate,
    duration: video.duration,
    height: video.videoHeight,
    resolution: video.videoResolution,
    playlist_title: video.playlistTitle,
    playlist_id: video.playlistId,
    playlist_index: video.playlistIndex,
    ext: "png",
  });
  const filename = sanitizeFilename(rendered).trim();
  return filename.toLowerCase().endsWith(".png")
    ? filename
    : `${filename || sanitizeFilename(title)}.png`;
};

/** 获取分辨率显示文本 */
const getResolutionLabel = (thumb: ThumbnailInfo): string => {
  if (thumb.width && thumb.height) return `${thumb.width} x ${thumb.height}`;
  if (thumb.resolution) return thumb.resolution;
  return t("common.unknown");
};

/** 画像URL・解像度が重複するサムネイルをまとめる */
const getVideoThumbnails = (video: ThumbnailVideoResult): ThumbnailInfo[] => {
  const seen = new Set<string>();
  const thumbnails = (video.thumbnails || [])
    .filter((item) => item.url)
    .filter((item) => {
      const key = item.width && item.height ? `${item.width}x${item.height}` : item.url;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort(
      (a, b) =>
        (b.width || 0) * (b.height || 0) - (a.width || 0) * (a.height || 0),
    );
  if (thumbnails.length === 0 && video.thumbnail) {
    return [{ url: video.thumbnail, id: video.id || "default" }];
  }
  return thumbnails;
};

const getHighestResolutionThumbnail = (
  video: ThumbnailVideoResult,
): ThumbnailInfo | undefined => {
  const thumbnails = getVideoThumbnails(video);
  if (thumbnails.some((thumb) => thumb.width && thumb.height)) return thumbnails[0];
  const youtubeFallback = getYoutubeFallbackThumbnails(video);
  if (youtubeFallback.length) return youtubeFallback[0];
  return thumbnails.find((thumb) => thumb.url === video.thumbnail) || thumbnails[0];
};

const getYoutubeFallbackThumbnails = (video: ThumbnailVideoResult): ThumbnailInfo[] => {
  try {
    const source = new URL(video.sourceUrl);
    if (!["youtube.com", "www.youtube.com", "m.youtube.com", "youtu.be"].includes(source.hostname)) {
      return [];
    }
    const pathSegments = source.pathname.split("/").filter(Boolean);
    const pathId =
      source.hostname === "youtu.be"
        ? pathSegments[0]
        : source.searchParams.get("v") || pathSegments[pathSegments.length - 1];
    const id = video.id || pathId || "";
    if (!/^[\w-]{11}$/.test(id)) return [];
    return ["maxresdefault", "sddefault", "hq720", "hqdefault", "mqdefault", "default"].map(
      (name) => ({
        id: `fallback-${name}`,
        url: `https://i.ytimg.com/vi/${id}/${name}.jpg`,
      }),
    );
  } catch {
    return [];
  }
};

const handleChooseThumbnailDownloadDir = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("toolbox.thumbnailChooseDownloadDir"),
    });
    if (typeof selected === "string") settingStore.thumbnailDownloadDir = selected;
  } catch (error: unknown) {
    showErrorDialog(String(error));
  }
};

/** 各動画の最高解像度サムネイルをダウンロードフォルダーへ直接保存 */
const saveHighestQualityThumbnails = async (videos: ThumbnailVideoResult[]) => {
  const videosWithoutThumbnail = videos.filter(
    (video) =>
      !getHighestResolutionThumbnail(video) && getYoutubeFallbackThumbnails(video).length === 0,
  );
  for (const video of videosWithoutThumbnail) {
    thumbnailErrors.value.push({ url: video.sourceUrl, error: t("toolbox.noThumbnailFound") });
  }
  const candidates = videos.flatMap((video, index) => {
    const videoThumbnails = getVideoThumbnails(video);
    const hasResolution = videoThumbnails.some((thumb) => thumb.width && thumb.height);
    const fallbackThumbnails = hasResolution ? [] : getYoutubeFallbackThumbnails(video);
    const thumbnail = getHighestResolutionThumbnail(video);
    const selectedThumbnail = thumbnail || fallbackThumbnails[0];
    return selectedThumbnail
      ? [{ video, index, thumbnail: selectedThumbnail, fallbackThumbnails }]
      : [];
  });
  if (candidates.length === 0) {
    window.$message.warning(t("toolbox.noThumbnailFound"));
    return;
  }

  highestQualitySaving.value = true;
  highestQualitySaved.value = 0;
  try {
    const directory =
      settingStore.thumbnailDownloadDir.trim() ||
      settingStore.downloadDir.trim() ||
      (await downloadDir());
    highestQualitySaveDir.value = directory;
    const usedNames = new Set<string>();
    for (const { video, thumbnail, fallbackThumbnails } of candidates) {
      let filename = getThumbnailFilename(video);
      const baseName = filename.slice(0, -".png".length);
      let duplicate = 2;
      while (usedNames.has(filename.toLowerCase())) {
        filename = `${baseName} (${duplicate}).png`;
        duplicate += 1;
      }
      usedNames.add(filename.toLowerCase());
      const filePath = await join(directory, filename);
      const alternatives = fallbackThumbnails.length ? fallbackThumbnails : [thumbnail];
      let saved = false;
      const failures: string[] = [];
      for (const alternative of alternatives) {
        try {
          await invoke("tool_save_thumbnail", {
            url: alternative.url,
            filePath,
            proxy: settingStore.proxy || null,
          });
          highestQualitySaved.value += 1;
          saved = true;
          break;
        } catch (error: unknown) {
          failures.push(String(error));
        }
      }
      if (!saved) thumbnailErrors.value.push({ url: video.sourceUrl, error: failures.join("; ") });
    }
  } catch (error: unknown) {
    showErrorDialog(String(error));
  } finally {
    highestQualitySaving.value = false;
  }
};

/** 获取视频信息并提取封面列表 */
const handleFetch = async () => {
  const submittedUrls = inputUrls.value.join("\n");
  currentFetchUrls.value = [...new Set(inputUrls.value)];
  const useHighestQuality = downloadHighestQuality.value;
  thumbnailVideos.value = [];
  thumbnailErrors.value = [];
  highestQualitySaved.value = 0;
  lastFetchUsedHighestQuality.value = useHighestQuality;
  autoSaveRunId.value = null;
  autoSavedRunId.value = null;
  try {
    const { cookieFile, cookieBrowser } = await videoStore.getCookieArgs();
    await start(submittedUrls, {
      cookieFile,
      cookieBrowser,
      proxy: settingStore.proxy || null,
    });
    if (useHighestQuality) autoSaveRunId.value = taskState.value?.runId || null;
  } catch (e: unknown) {
    autoSaveRunId.value = null;
    const msg = String(e);
    if (/err_ytdlp_not_installed/.test(msg)) {
      statusStore.showYtdlpSetupModal = true;
    } else if (/sign in|cookies/i.test(msg)) {
      window.$message.warning(t("cookie.verificationDesc"));
      statusStore.showCookieModal = true;
    } else {
      showErrorDialog(msg);
    }
  }
};

const handleStop = async () => {
  try {
    await cancel();
  } catch (e: unknown) {
    showErrorDialog(String(e));
  }
};

watch(taskState, (state) => {
  if (state?.url) url.value = state.url;
});

watch(
  [taskResult, taskResultMeta, taskState, autoSaveRunId],
  async ([result, resultMeta, state, saveRunId]) => {
    if (!result) return;
    if (lastFetchUsedHighestQuality.value) {
      // The task store restores the previous result when the page opens. Only
      // process the result belonging to the latest auto-download run.
      if (
        !saveRunId ||
        state?.status !== "completed" ||
        state.runId !== saveRunId ||
        resultMeta?.runId !== saveRunId ||
        autoSavedRunId.value === saveRunId
      ) {
        return;
      }
      autoSavedRunId.value = saveRunId;
    }
    if (Array.isArray(result.items)) {
      thumbnailVideos.value = result.items;
      thumbnailErrors.value = result.errors || [];
      highestQualityVideoTotal.value = result.expectedCount ?? result.items.length;
      if (lastFetchUsedHighestQuality.value) {
        const completedUrls = new Set([
          ...thumbnailVideos.value.map((video) => video.sourceUrl),
          ...thumbnailErrors.value.map((failure) => failure.url),
        ]);
        for (const sourceUrl of currentFetchUrls.value) {
          if (!completedUrls.has(sourceUrl)) {
            thumbnailErrors.value.push({
              url: sourceUrl,
              error: t("toolbox.thumbnailNoResultForUrl"),
            });
          }
        }
      }
    } else {
      // 以前の単体取得結果を再表示できるように旧形式も読み込む。
      const legacyResult = result as unknown as Partial<ThumbnailVideoResult>;
      thumbnailVideos.value =
        legacyResult.title || legacyResult.thumbnail || legacyResult.thumbnails?.length
          ? [
              {
                id: legacyResult.id || "",
                title: legacyResult.title || "",
                thumbnail: legacyResult.thumbnail || "",
                thumbnails: legacyResult.thumbnails || [],
                playlistTitle: "",
                playlistIndex: null,
                sourceUrl: legacyResult.sourceUrl || "",
              },
            ]
          : [];
      thumbnailErrors.value = [];
    }
    if (lastFetchUsedHighestQuality.value) {
      await saveHighestQualityThumbnails(thumbnailVideos.value);
    } else if (thumbnailVideos.value.every((video) => getVideoThumbnails(video).length === 0)) {
      window.$message.warning(t("toolbox.noThumbnailFound"));
    }
  },
);

/** 另存为 */
const handleSave = async (thumb: ThumbnailInfo, video: ThumbnailVideoResult) => {
  const ext = getExtFromUrl(thumb.url);
  const defaultName = getThumbnailFilename(video);

  const filePath = await save({
    title: t("toolbox.saveThumbnail"),
    defaultPath: defaultName,
    filters: [{ name: t("toolbox.imageFiles"), extensions: [ext] }],
  });
  if (!filePath) return;

  const id = `${video.id}:${thumb.id || thumb.url}`;
  savingId.value = id;
  try {
    await invoke("tool_save_thumbnail", {
      url: thumb.url,
      filePath,
      proxy: settingStore.proxy || null,
    });
    window.$message.success(t("toolbox.thumbnailSaved"));
  } catch (e: unknown) {
    window.$message.error(t("common.saveFailed", { e }));
  } finally {
    savingId.value = null;
  }
};
</script>

<template>
  <n-flex vertical :size="12">
    <n-flex align="center" :size="8">
      <n-button strong secondary size="small" @click="goBack">
        <template #icon>
          <n-icon><icon-mdi-arrow-left /></n-icon>
        </template>
        {{ $t("common.back") }}
      </n-button>
      <n-text strong style="font-size: 15px">{{ $t("toolbox.thumbnailTitle") }}</n-text>
    </n-flex>

    <n-card size="small">
      <n-flex vertical :size="12">
        <n-text depth="3" style="font-size: 13px">
          {{ $t("toolbox.thumbnailPageDesc") }}
        </n-text>
        <n-input
          v-model:value="url"
          type="textarea"
          :autosize="{ minRows: 2, maxRows: 6 }"
          :placeholder="$t('toolbox.thumbnailUrlsPlaceholder')"
          :disabled="loading || highestQualitySaving"
        />
        <n-flex align="center" :size="8" :wrap="false">
          <n-input
            v-model:value="settingStore.thumbnailDownloadDir"
            :placeholder="
              $t('toolbox.thumbnailDownloadDirPlaceholder', {
                path: settingStore.downloadDir || $t('toolbox.systemDownloadDir'),
              })
            "
            :disabled="loading || highestQualitySaving"
            clearable
          />
          <n-button
            :disabled="loading || highestQualitySaving"
            @click="handleChooseThumbnailDownloadDir"
          >
            <template #icon>
              <n-icon><icon-mdi-folder-open-outline /></n-icon>
            </template>
            {{ $t("toolbox.thumbnailChooseDownloadDir") }}
          </n-button>
        </n-flex>
        <n-text depth="3" style="font-size: 12px">
          {{ $t("toolbox.thumbnailFilenameTemplateHint") }}
        </n-text>
        <n-checkbox
          v-model:checked="downloadHighestQuality"
          :disabled="loading || highestQualitySaving"
        >
          {{ $t("toolbox.thumbnailHighestQualityOption") }}
        </n-checkbox>
        <n-text v-if="downloadHighestQuality" depth="3" style="font-size: 12px">
          {{ $t("toolbox.thumbnailHighestQualityHint") }}
        </n-text>
        <ToolTaskActions
          :loading="loading"
          :disabled="!urlValid || highestQualitySaving"
          @fetch="handleFetch"
          @stop="handleStop"
        >
          <template #icon>
            <n-icon><icon-mdi-image-search /></n-icon>
          </template>
          {{
            $t(
              downloadHighestQuality
                ? "toolbox.fetchAndAutoDownloadThumbnails"
                : "toolbox.fetchThumbnails",
            )
          }}
        </ToolTaskActions>
      </n-flex>
    </n-card>

    <n-alert
      v-if="highestQualitySaving"
      type="info"
      :title="$t('toolbox.thumbnailAutoSaving', { n: thumbnailVideoCount })"
    />

    <n-alert
      v-else-if="lastFetchUsedHighestQuality && highestQualitySaved > 0"
      type="success"
      :title="
        $t('toolbox.thumbnailAutoSaved', {
          n: highestQualitySaved,
          total: highestQualityVideoTotal,
        })
      "
    >
      {{ highestQualitySaveDir }}
    </n-alert>

    <n-alert
      v-if="
        lastFetchUsedHighestQuality &&
        !loading &&
        !highestQualitySaving &&
        highestQualityVideoTotal > thumbnailVideoCount
      "
      type="warning"
      :title="
        $t('toolbox.thumbnailPlaylistIncomplete', {
          expected: highestQualityVideoTotal,
          found: thumbnailVideoCount,
          missing: highestQualityVideoTotal - thumbnailVideoCount,
        })
      "
    />

    <n-alert
      v-if="thumbnailErrors.length"
      type="warning"
      :title="$t('toolbox.thumbnailErrorsTitle', { n: thumbnailErrors.length })"
    >
      <n-flex vertical :size="4">
        <n-text v-for="failure in thumbnailErrors" :key="failure.url" depth="3">
          {{ failure.url }} — {{ failure.error }}
        </n-text>
      </n-flex>
    </n-alert>

    <n-card
      v-if="thumbnailVideoCount && !lastFetchUsedHighestQuality"
      size="small"
      :title="$t('toolbox.thumbnailVideoCount', { n: thumbnailVideoCount })"
    >
      <n-flex vertical :size="10">
        <n-card
          v-for="video in thumbnailVideos"
          :key="video.id || `${video.sourceUrl}-${video.playlistIndex}`"
          size="small"
          :title="
            video.playlistIndex !== null
              ? $t('toolbox.thumbnailPlaylistItemTitle', {
                  playlist: video.playlistTitle,
                  index: video.playlistIndex,
                  title: video.title,
                })
              : video.title
          "
        >
          <n-list v-if="getVideoThumbnails(video).length" hoverable clickable bordered>
            <n-list-item
              v-for="thumb in getVideoThumbnails(video)"
              :key="thumb.id || thumb.url"
            >
              <n-flex align="center" :size="12" :wrap="false">
                <ThumbnailPreview
                  :src="thumb.url"
                  lazy
                  preview-disabled
                  width="120"
                  height="68"
                  object-fit="cover"
                  style="border-radius: 4px; flex-shrink: 0"
                />
                <n-flex align="center" justify="space-between" style="flex: 1; min-width: 0">
                  <n-text style="font-size: 13px">{{ getResolutionLabel(thumb) }}</n-text>
                  <n-button
                    size="small"
                    :loading="savingId === `${video.id}:${thumb.id || thumb.url}`"
                    @click="handleSave(thumb, video)"
                  >
                    <template #icon>
                      <n-icon><icon-mdi-content-save-outline /></n-icon>
                    </template>
                    {{ $t("common.saveAs") }}
                  </n-button>
                </n-flex>
              </n-flex>
            </n-list-item>
          </n-list>
          <n-text v-else depth="3">{{ $t("toolbox.noThumbnailFound") }}</n-text>
        </n-card>
      </n-flex>
    </n-card>
  </n-flex>
</template>
