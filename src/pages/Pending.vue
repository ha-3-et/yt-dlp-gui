<script setup lang="ts">
import { formatFileSize } from "@/utils/format";
import {
  estimateFileSize,
  normalizeAudioFormat,
  normalizeVideoFormat,
} from "@/utils/normalizer";
import { useVideoStore } from "@/stores/video";
import { useSettingStore } from "@/stores/setting";
import { usePendingStore } from "@/stores/pending";
import { useDownloadLauncher } from "@/composables/useDownloadLauncher";
import { getPlaylistEntryFormats } from "@/utils/playlist";
import { useI18n } from "vue-i18n";
import type { PlaylistEntry, VideoInfo } from "@/types";

const { t } = useI18n();
const router = useRouter();
const videoStore = useVideoStore();
const settingStore = useSettingStore();
const pendingStore = usePendingStore();
const { launchDownload } = useDownloadLauncher();

const activeItem = computed(() => pendingStore.activeItem);
const playlistIndex = (entry: PlaylistEntry, index: number) => entry.playlist_index ?? index + 1;
const startingDownload = ref(false);

/**
 * 待下载页偏好型选项持久化
 */
const persistedOptionSnapshot = computed(() => {
  const item = activeItem.value;
  if (!item) return null;
  return {
    embedSubs: item.embedSubs,
    embedThumbnail: item.embedThumbnail,
    writeThumbnail: item.writeThumbnail,
    writeDescription: item.writeDescription,
    embedMetadata: item.embedMetadata,
    embedChapters: item.embedChapters,
    sponsorblockRemove: item.sponsorblockRemove,
    extractAudio: item.extractAudio,
    audioConvertFormat: item.audioConvertFormat,
    noMerge: item.noMerge,
    recodeFormat: item.recodeFormat,
    remuxFormat: item.remuxFormat,
    limitRate: item.limitRate,
    customArgs: item.customArgs,
  };
});

watch(persistedOptionSnapshot, (snapshot) => {
  if (!snapshot) return;
  settingStore.defaultEmbedSubs = snapshot.embedSubs;
  settingStore.defaultEmbedThumbnail = snapshot.embedThumbnail;
  settingStore.defaultWriteThumbnail = snapshot.writeThumbnail;
  settingStore.defaultWriteDescription = snapshot.writeDescription;
  settingStore.defaultEmbedMetadata = snapshot.embedMetadata;
  settingStore.defaultEmbedChapters = snapshot.embedChapters;
  settingStore.defaultSponsorblockRemove = snapshot.sponsorblockRemove;
  settingStore.defaultExtractAudio = snapshot.extractAudio;
  settingStore.defaultAudioConvertFormat = snapshot.audioConvertFormat;
  settingStore.defaultNoMerge = snapshot.noMerge;
  settingStore.defaultRecodeFormat = snapshot.recodeFormat;
  settingStore.defaultRemuxFormat = snapshot.remuxFormat;
  settingStore.defaultLimitRate = snapshot.limitRate;
  settingStore.defaultCustomArgs = snapshot.customArgs;
});

const estimatedSize = computed(() => {
  const item = activeItem.value;
  if (!item) return 0;
  let total = 0;
  if (item.isPlaylist && item.playlistIndividualFormats) {
    for (const index of item.selectedPlaylistItems) {
      const position = item.playlistEntries.findIndex(
        (entry, entryPosition) => playlistIndex(entry, entryPosition) === index,
      );
      const entry = item.playlistEntries[position];
      if (!entry) continue;
      const formats = getPlaylistEntryFormats(entry, item.videoFormats, item.audioFormats);
      const selection = item.playlistFormatSelections?.[index];
      const duration = entry.duration || 0;
      if (item.downloadMode !== "audio" && !item.extractAudio) {
        const video = formats.videoFormats.find(
          (format) => format.format_id === selection?.videoFormat,
        );
        if (video) total += estimateFileSize(video, duration).size;
      }
      if (item.downloadMode !== "video" || item.extractAudio) {
        const audio = formats.audioFormats.find(
          (format) => format.format_id === selection?.audioFormat,
        );
        if (audio) total += estimateFileSize(audio, duration).size;
      }
    }
    return total;
  }
  const duration = item.videoInfo?.duration || 0;
  if (item.downloadMode !== "audio") {
    const vf = item.videoFormats.find((f) => f.format_id === item.selectedVideoFormat);
    if (vf) {
      const { size } = estimateFileSize(vf, duration);
      total += size;
    }
  }
  if (item.downloadMode !== "video") {
    const selectedAudio = item.selectedAudioFormat.split("+");
    for (const af of item.audioFormats.filter((f) => selectedAudio.includes(f.format_id))) {
      const { size } = estimateFileSize(af, duration);
      total += size;
    }
  }
  return total;
});

const estimatedSizeText = computed(() => {
  if (!estimatedSize.value) return t("common.unknown");
  return formatFileSize(estimatedSize.value);
});

const playlistSelection = (entry: PlaylistEntry, index: number) =>
  activeItem.value?.playlistFormatSelections?.[playlistIndex(entry, index)] ?? {
    videoFormat: "",
    audioFormat: "",
  };

const setPlaylistFormat = (
  entry: PlaylistEntry,
  index: number,
  kind: "videoFormat" | "audioFormat",
  value: string | null,
) => {
  const item = activeItem.value;
  if (!item) return;
  const key = playlistIndex(entry, index);
  item.playlistFormatSelections ??= {};
  item.playlistFormatSelections[key] ??= { videoFormat: "", audioFormat: "" };
  item.playlistFormatSelections[key][kind] = value || "";
};

const getPlaylistFormatOptions = (entry: PlaylistEntry, kind: "video" | "audio") => {
  const item = activeItem.value;
  if (!item) return [];
  const formats = getPlaylistEntryFormats(entry, item.videoFormats, item.audioFormats);
  const selectedFormats = kind === "video" ? formats.videoFormats : formats.audioFormats;
  return selectedFormats.map((format) => {
    const parts =
      kind === "video"
        ? (() => {
            const normalized = normalizeVideoFormat(format, entry.duration || 0);
            return [
              normalized.resolutionLabel,
              normalized.fpsLabel,
              normalized.codec,
              normalized.container,
              normalized.filesizeLabel,
              `#${normalized.formatId}`,
            ];
          })()
        : (() => {
            const normalized = normalizeAudioFormat(format, entry.duration || 0);
            return [
              normalized.languageLabel || normalized.language,
              normalized.bitrateLabel,
              normalized.codec,
              normalized.channelsLabel,
              normalized.container,
              normalized.filesizeLabel,
              `#${normalized.formatId}`,
            ];
          })();
    return { label: parts.filter(Boolean).join(" · "), value: format.format_id };
  });
};

const launchPlaylistWithIndividualFormats = async (item: NonNullable<typeof activeItem.value>) => {
  const selectedItems = [...item.selectedPlaylistItems];
  const remainingItems: number[] = [];
  for (let i = 0; i < selectedItems.length; i += 1) {
    const index = selectedItems[i];
    const entryIndex = item.playlistEntries.findIndex(
      (entry, entryPosition) => playlistIndex(entry, entryPosition) === index,
    );
    const entry = item.playlistEntries[entryIndex];
    if (!entry) {
      remainingItems.push(index);
      continue;
    }
    const formats = getPlaylistEntryFormats(entry, item.videoFormats, item.audioFormats);
    const selection = item.playlistFormatSelections?.[index] ?? {
      videoFormat: item.selectedVideoFormat,
      audioFormat: item.selectedAudioFormat,
    };
    const result = await launchDownload({
      ...item,
      videoInfo: {
        ...item.videoInfo,
        title: entry.title || item.videoInfo.title,
        thumbnail: entry.thumbnail || item.videoInfo.thumbnail,
        duration: entry.duration || 0,
      },
      videoFormats: formats.videoFormats,
      audioFormats: formats.audioFormats,
      selectedPlaylistItems: [index],
      selectedVideoFormat: selection.videoFormat,
      selectedAudioFormat: selection.audioFormat,
    });
    if (result !== "started" && result !== "queued") {
      if (result === "missing-directory") {
        dirCardRef.value?.scrollIntoView({ behavior: "smooth", block: "center" });
      }
      remainingItems.push(index, ...selectedItems.slice(i + 1));
      break;
    }
  }
  item.selectedPlaylistItems = remainingItems;
};

const dirCardRef = ref<HTMLElement | null>(null);

const tabLabel = (title: string): string => {
  if (!title) return t("detail.unknownVideo");
  if (title.length > 12) return title.slice(0, 10) + "…";
  return title;
};

const handleTabClose = (name: string | number) => {
  pendingStore.remove(String(name));
};

const handleTabAdd = () => {
  router.push({ name: "home" });
};

const handleBackToHome = () => {
  router.push({ name: "home" });
};

/** 重新获取当前项视频信息 */
const handleRefresh = async () => {
  const item = activeItem.value;
  if (!item) return;
  const data = await videoStore.fetchVideoInfo(item.url, { customArgs: item.customArgs });
  if (data) {
    pendingStore.refresh(item.id, data);
    window.$message.success(t("detail.refreshSuccess"));
  }
};

/** 开始下载当前项 */
const handleDownload = async () => {
  const item = activeItem.value;
  if (!item || startingDownload.value) return;
  startingDownload.value = true;
  try {
    if (item.isPlaylist && item.playlistIndividualFormats) {
      await launchPlaylistWithIndividualFormats(item);
      if (item.selectedPlaylistItems.length > 0) return;
      pendingStore.remove(item.id);
      if (pendingStore.items.length === 0) {
        router.push({ name: "downloads" });
      }
      return;
    }

    const result = await launchDownload(item);
    if (result === "missing-directory") {
      dirCardRef.value?.scrollIntoView({ behavior: "smooth", block: "center" });
    } else if (result === "started" || result === "queued") {
      pendingStore.remove(item.id);
      if (pendingStore.items.length === 0) {
        router.push({ name: "downloads" });
      }
    }
  } finally {
    startingDownload.value = false;
  }
};
</script>

<template>
  <div class="pending-page">
    <template v-if="pendingStore.items.length > 0">
      <n-tabs
        v-model:value="pendingStore.activeId"
        type="card"
        size="small"
        closable
        addable
        class="tabs-bar"
        @close="handleTabClose"
        @add="handleTabAdd"
      >
        <template #prefix>
          <n-button size="small" strong secondary circle @click="handleBackToHome">
            <template #icon>
              <n-icon><icon-mdi-arrow-left /></n-icon>
            </template>
          </n-button>
        </template>
        <n-tab-pane
          v-for="item in pendingStore.items"
          :key="item.id"
          :name="item.id"
          :tab="tabLabel(item.videoInfo.title)"
          display-directive="show"
        />
      </n-tabs>

      <div v-if="activeItem" :key="activeItem.id" class="pending-content">
        <n-flex :size="8" align="center" :wrap="false" style="margin-bottom: 16px">
          <n-input
            :value="activeItem.url"
            :placeholder="$t('detail.videoLink')"
            size="small"
            round
            readonly
            style="flex: 1; min-width: 0"
          />
          <n-button
            size="small"
            strong
            secondary
            round
            :loading="videoStore.fetching"
            @click="handleRefresh"
          >
            <template #icon>
              <n-icon><icon-mdi-refresh /></n-icon>
            </template>
          </n-button>
        </n-flex>

        <VideoInfoCard
          :video-info="activeItem.videoInfo as VideoInfo"
          :is-playlist="activeItem.isPlaylist"
          :playlist-count="activeItem.playlistEntries.length"
          class="section-card"
        />

        <n-card
          v-if="activeItem.isPlaylist && activeItem.playlistEntries.length > 0"
          size="small"
          class="section-card"
        >
          <template #header>
            <n-flex align="center" :size="8">
              <n-icon size="16"><icon-mdi-playlist-play /></n-icon>
              <span>{{ $t("detail.playlist") }}</span>
              <n-tag size="small" round :bordered="false" type="info">
                {{ activeItem.selectedPlaylistItems.length }} /
                {{ activeItem.playlistEntries.length }}
              </n-tag>
            </n-flex>
          </template>
          <template #header-extra>
            <n-flex align="center" :size="8" :wrap="true">
              <n-flex align="center" :size="6">
                <n-switch
                  :value="Boolean(activeItem.playlistIndividualFormats)"
                  size="small"
                  @update:value="activeItem.playlistIndividualFormats = $event"
                />
                <n-text depth="2" style="font-size: 12px">
                  {{ $t("detail.individualPlaylistFormats") }}
                </n-text>
              </n-flex>
              <n-button
                size="tiny"
                secondary
                @click="
                  activeItem.selectedPlaylistItems = activeItem.playlistEntries.map(
                    (entry, i) => entry.playlist_index ?? i + 1,
                  )
                "
              >
                {{ $t("common.selectAll") }}
              </n-button>
              <n-button size="tiny" secondary @click="activeItem.selectedPlaylistItems = []">
                {{ $t("common.deselectAll") }}
              </n-button>
            </n-flex>
          </template>
          <n-alert
            v-if="activeItem.playlistIndividualFormats"
            type="info"
            :bordered="false"
            style="margin-bottom: 8px"
          >
            {{ $t("detail.individualPlaylistFormatsHint") }}
          </n-alert>
          <n-checkbox-group v-model:value="activeItem.selectedPlaylistItems">
            <n-flex vertical :size="6">
              <div
                v-for="(entry, index) in activeItem.playlistEntries"
                :key="entry.id"
                class="playlist-entry"
              >
                <n-checkbox
                  :value="playlistIndex(entry, index)"
                  :label="`P${playlistIndex(entry, index)} ${entry.title}`"
                />
                <n-flex
                  v-if="
                    activeItem.playlistIndividualFormats &&
                    activeItem.selectedPlaylistItems.includes(playlistIndex(entry, index))
                  "
                  :size="8"
                  class="playlist-entry-formats"
                >
                  <n-select
                    v-if="activeItem.downloadMode !== 'audio' && !activeItem.extractAudio"
                    :value="playlistSelection(entry, index).videoFormat"
                    :options="getPlaylistFormatOptions(entry, 'video')"
                    :placeholder="$t('detail.video')"
                    size="small"
                    style="flex: 1; min-width: 0"
                    @update:value="setPlaylistFormat(entry, index, 'videoFormat', $event)"
                  />
                  <n-select
                    v-if="activeItem.downloadMode !== 'video' || activeItem.extractAudio"
                    :value="playlistSelection(entry, index).audioFormat"
                    :options="getPlaylistFormatOptions(entry, 'audio')"
                    :placeholder="$t('detail.audio')"
                    size="small"
                    style="flex: 1; min-width: 0"
                    @update:value="setPlaylistFormat(entry, index, 'audioFormat', $event)"
                  />
                </n-flex>
              </div>
            </n-flex>
          </n-checkbox-group>
        </n-card>

        <DownloadOptionsCard
          v-model:download-mode="activeItem.downloadMode"
          v-model:selected-video-format="activeItem.selectedVideoFormat"
          v-model:selected-audio-format="activeItem.selectedAudioFormat"
          :video-formats="activeItem.videoFormats"
          :extract-audio="activeItem.extractAudio"
          :audio-formats="activeItem.audioFormats"
          :video-info="activeItem.videoInfo as VideoInfo"
          class="section-card"
        />

        <SubtitleCard
          v-model:selected-subtitles="activeItem.selectedSubtitles"
          :video-info="activeItem.videoInfo as VideoInfo"
          class="section-card"
        />

        <ExtraOptionsCard
          v-model:start-time="activeItem.startTime"
          v-model:end-time="activeItem.endTime"
          v-model:force-keyframes-at-cuts="activeItem.forceKeyframesAtCuts"
          v-model:embed-subs="activeItem.embedSubs"
          v-model:embed-thumbnail="activeItem.embedThumbnail"
          v-model:write-thumbnail="activeItem.writeThumbnail"
          v-model:write-description="activeItem.writeDescription"
          v-model:embed-metadata="activeItem.embedMetadata"
          v-model:embed-chapters="activeItem.embedChapters"
          v-model:sponsorblock-remove="activeItem.sponsorblockRemove"
          v-model:extract-audio="activeItem.extractAudio"
          v-model:audio-convert-format="activeItem.audioConvertFormat"
          v-model:no-merge="activeItem.noMerge"
          v-model:recode-format="activeItem.recodeFormat"
          v-model:remux-format="activeItem.remuxFormat"
          v-model:limit-rate="activeItem.limitRate"
          v-model:ffmpeg-args="activeItem.ffmpegArgs"
          v-model:custom-args="activeItem.customArgs"
          :video-info="activeItem.videoInfo as VideoInfo"
          class="section-card"
        />

        <div ref="dirCardRef" class="section-card">
          <DownloadDirCard />
        </div>

        <DownloadBar
          :estimated-size-text="estimatedSizeText"
          :loading="startingDownload"
          :disabled="
            startingDownload ||
            (activeItem.isPlaylist && !activeItem.selectedPlaylistItems.length)
          "
          @download="handleDownload"
        />
      </div>
    </template>

    <n-empty v-else :description="$t('pending.empty')" class="empty-state">
      <template #extra>
        <n-button type="primary" round @click="handleBackToHome">
          <template #icon>
            <n-icon><icon-mdi-magnify /></n-icon>
          </template>
          {{ $t("pending.goParse") }}
        </n-button>
      </template>
    </n-empty>
  </div>
</template>

<style scoped lang="scss">
.pending-page {
  position: relative;
  height: 100%;
  overflow-y: auto;
  padding: 16px;

  .tabs-bar {
    margin-bottom: 12px;

    :deep(.n-tabs-nav__prefix) {
      padding-right: 8px;
    }

    :deep(.n-tabs-tab) {
      padding-left: 10px;
      padding-right: 6px;
    }
  }

  .section-card {
    margin-bottom: 16px;
  }

  .playlist-entry {
    min-width: 0;
  }

  .playlist-entry-formats {
    margin: 4px 0 4px 26px;
  }

  .pending-content {
    padding-bottom: 96px;
  }

  .empty-state {
    margin-top: 120px;
  }
}
</style>
