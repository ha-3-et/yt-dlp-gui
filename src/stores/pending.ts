import { defineStore } from "pinia";
import { useSettingStore } from "@/stores/setting";
import { getCodecKey } from "@/utils/formats";
import { getPlaylistEntryFormats } from "@/utils/playlist";
import type {
  FetchedVideoData,
  PendingItem,
  PlaylistEntry,
  PlaylistFormatSelection,
  VideoFormat,
} from "@/types";

const generateId = () => `pd_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;

const selectVideoFormat = (
  formats: VideoFormat[],
  preferredCodec: string,
  maxHeight?: number,
): string => {
  const withinHeight = maxHeight
    ? formats.filter((format) => format.height != null && format.height <= maxHeight)
    : formats;
  const candidates = withinHeight.length ? withinHeight : formats;
  return (
    candidates.find((format) => getCodecKey(format.vcodec) === preferredCodec)?.format_id ??
    candidates[0]?.format_id ??
    ""
  );
};

const selectPlaylistFormat = (
  entry: PlaylistEntry,
  videoFormats: VideoFormat[],
  audioFormats: VideoFormat[],
  preferredVideoCodec: string,
  preferredAudioCodec: string,
): PlaylistFormatSelection => {
  const formats = getPlaylistEntryFormats(entry, videoFormats, audioFormats);
  return {
    videoFormat: selectVideoFormat(formats.videoFormats, preferredVideoCodec),
    audioFormat:
      formats.audioFormats.find(
        (format) => getCodecKey(format.acodec) === preferredAudioCodec,
      )?.format_id ??
      formats.audioFormats[0]?.format_id ??
      "",
  };
};

/**
 * 根据解析后的视频元数据创建新的待下载任务配置对象
 *
 * @param data 解析完成的视频数据
 * @param quick 是否采用快速下载预设配置
 * @returns 初始化后的待下载项
 */
export const createPendingItem = (data: FetchedVideoData, quick = false): PendingItem => {
  const settingStore = useSettingStore();
  const maxHeight = quick ? settingStore.quickMaxHeight : undefined;
  const selectedVideoFormat = selectVideoFormat(
    data.videoFormats,
    settingStore.preferredVideoCodec,
    maxHeight,
  );
  const selectedAudioFormat =
    data.audioFormats.find(
      (format) => getCodecKey(format.acodec) === settingStore.preferredAudioCodec,
    )?.format_id ??
    data.audioFormats[0]?.format_id ??
    "";
  const playlistFormatSelections = Object.fromEntries(
    data.playlistEntries.map((entry, index) => [
      entry.playlist_index ?? index + 1,
      selectPlaylistFormat(
        entry,
        data.videoFormats,
        data.audioFormats,
        settingStore.preferredVideoCodec,
        settingStore.preferredAudioCodec,
      ),
    ]),
  );
  return {
    ...data,
    id: generateId(),
    createdAt: Date.now(),
    selectedPlaylistItems: data.isPlaylist
      ? data.playlistEntries.map((entry, i) => entry.playlist_index ?? i + 1)
      : [],
    playlistIndividualFormats: false,
    playlistFormatSelections,
    downloadMode: quick ? settingStore.quickDownloadMode : settingStore.defaultDownloadMode,
    selectedVideoFormat,
    selectedAudioFormat,
    startTime: null,
    endTime: null,
    forceKeyframesAtCuts: false,
    embedSubs: quick ? false : settingStore.defaultEmbedSubs,
    embedThumbnail: quick ? settingStore.quickEmbedThumbnail : settingStore.defaultEmbedThumbnail,
    writeThumbnail: quick ? settingStore.quickWriteThumbnail : settingStore.defaultWriteThumbnail,
    writeDescription: quick
      ? settingStore.quickWriteDescription
      : settingStore.defaultWriteDescription,
    embedMetadata: quick ? settingStore.quickEmbedMetadata : settingStore.defaultEmbedMetadata,
    embedChapters: quick ? settingStore.quickEmbedChapters : settingStore.defaultEmbedChapters,
    sponsorblockRemove: quick
      ? settingStore.quickSponsorblockRemove
      : settingStore.defaultSponsorblockRemove,
    extractAudio: quick ? false : settingStore.defaultExtractAudio,
    audioConvertFormat: quick ? "" : settingStore.defaultAudioConvertFormat,
    noMerge: quick ? settingStore.quickNoMerge : settingStore.defaultNoMerge,
    recodeFormat: quick ? settingStore.quickRecodeFormat : settingStore.defaultRecodeFormat,
    remuxFormat: quick ? settingStore.quickRemuxFormat : settingStore.defaultRemuxFormat,
    limitRate: quick ? settingStore.quickLimitRate : settingStore.defaultLimitRate,
    ffmpegArgs: quick ? settingStore.quickFfmpegArgs : settingStore.defaultFfmpegArgs,
    customArgs: quick
      ? settingStore.quickCustomArgs || settingStore.defaultCustomArgs
      : settingStore.defaultCustomArgs,
    selectedSubtitles: [],
    liveFromStart: data.videoInfo.is_live === true || data.videoInfo.live_status === "is_live",
  };
};

/**
 * 待下载任务配置队列 Store
 *
 * 负责在正式开始下载前暂存已解析的视频项、用户参数微调与多任务切换。
 */
export const usePendingStore = defineStore("pending", () => {
  const items = ref<PendingItem[]>([]);
  const activeId = ref<string>("");

  const activeItem = computed<PendingItem | null>(
    () => items.value.find((i) => i.id === activeId.value) ?? null,
  );

  /**
   * 将解析出的视频数据加入待下载项列表，并将其激活为当前编辑项
   *
   * @param data 解析出的视频数据
   * @returns 新增待下载项的唯一 ID
   */
  const add = (data: FetchedVideoData): string => {
    const item = createPendingItem(data);
    items.value.push(item);
    activeId.value = item.id;
    return item.id;
  };

  /**
   * 移除指定 ID 的待下载项，并自动激活临近项
   *
   * @param id 待删除项 ID
   */
  const remove = (id: string): void => {
    const idx = items.value.findIndex((i) => i.id === id);
    if (idx === -1) return;
    items.value.splice(idx, 1);
    if (activeId.value === id) {
      const next = items.value[idx] ?? items.value[idx - 1] ?? items.value[0];
      activeId.value = next ? next.id : "";
    }
  };

  /**
   * 刷新当前待下载项的元数据（重新解析后覆盖音视频流信息，保留用户已修改的选项）
   *
   * @param id 待下载项 ID
   * @param data 重新解析获得的视频数据
   */
  const refresh = (id: string, data: FetchedVideoData): void => {
    const item = items.value.find((i) => i.id === id);
    if (!item) return;
    const settingStore = useSettingStore();
    const wasPlaylist = item.isPlaylist;
    const selectedVideo = item.selectedVideoFormat;
    const selectedAudio = item.selectedAudioFormat.split("+");
    const previousPlaylistFormats = item.playlistFormatSelections ?? {};
    item.url = data.url;
    item.videoInfo = data.videoInfo;
    item.videoFormats = data.videoFormats;
    item.audioFormats = data.audioFormats;
    item.isPlaylist = data.isPlaylist;
    item.playlistEntries = data.playlistEntries;
    item.playlistFormatSelections = Object.fromEntries(
      data.playlistEntries.map((entry, index) => {
        const playlistIndex = entry.playlist_index ?? index + 1;
        const previous = previousPlaylistFormats[playlistIndex];
        const defaults = selectPlaylistFormat(
          entry,
          data.videoFormats,
          data.audioFormats,
          settingStore.preferredVideoCodec,
          settingStore.preferredAudioCodec,
        );
        const formats = getPlaylistEntryFormats(entry, data.videoFormats, data.audioFormats);
        return [
          playlistIndex,
          {
            videoFormat: formats.videoFormats.some(
              (format) => format.format_id === previous?.videoFormat,
            )
              ? previous.videoFormat
              : defaults.videoFormat,
            audioFormat: formats.audioFormats.some(
              (format) => format.format_id === previous?.audioFormat,
            )
              ? previous.audioFormat
              : defaults.audioFormat,
          },
        ];
      }),
    );
    const indices = data.playlistEntries.map((entry, i) => entry.playlist_index ?? i + 1);
    item.selectedPlaylistItems = data.isPlaylist
      ? wasPlaylist
        ? item.selectedPlaylistItems.filter((index) => indices.includes(index))
        : indices
      : [];
    item.selectedVideoFormat = data.videoFormats.some(
      (format) => format.format_id === selectedVideo,
    )
      ? selectedVideo
      : (data.videoFormats[0]?.format_id ?? "");
    const retainedAudio = selectedAudio.filter((id) =>
      data.audioFormats.some((format) => format.format_id === id),
    );
    item.selectedAudioFormat = retainedAudio.length
      ? retainedAudio.join("+")
      : (data.audioFormats[0]?.format_id ?? "");
  };

  /**
   * 清空全部待下载项
   */
  const clear = (): void => {
    items.value = [];
    activeId.value = "";
  };

  return {
    items,
    activeId,
    activeItem,
    add,
    remove,
    refresh,
    clear,
  };
});
