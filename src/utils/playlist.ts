import { compareAudioFormats } from "@/utils/formats";
import { compareVideoFormats } from "@/utils/normalizer";
import type { PlaylistEntry, VideoFormat } from "@/types";

export const filterPlaylistEntries = <T extends { playlist_index?: number | null }>(
  entries: Array<T | null | undefined>,
): Array<T & { playlist_index: number }> =>
  entries.flatMap((entry, index) =>
    entry == null ? [] : [{ ...entry, playlist_index: entry.playlist_index ?? index + 1 }],
  );

export const getPlaylistEntryFormats = (
  entry: PlaylistEntry,
  fallbackVideoFormats: VideoFormat[] = [],
  fallbackAudioFormats: VideoFormat[] = [],
): { videoFormats: VideoFormat[]; audioFormats: VideoFormat[] } => {
  const formats = entry.formats?.length
    ? entry.formats
    : [...fallbackVideoFormats, ...fallbackAudioFormats];
  const videoOnlyFormats = formats
    .filter(
      (format) =>
        format.vcodec &&
        format.vcodec !== "none" &&
        (!format.acodec || format.acodec === "none"),
    )
    .sort(compareVideoFormats);
  const combinedFormats = formats
    .filter(
      (format) =>
        format.vcodec &&
        format.vcodec !== "none" &&
        format.acodec &&
        format.acodec !== "none",
    )
    .sort(compareVideoFormats);
  const audioFormats = formats
    .filter(
      (format) =>
        format.acodec &&
        format.acodec !== "none" &&
        (!format.vcodec || format.vcodec === "none"),
    )
    .sort(compareAudioFormats);
  return {
    videoFormats: videoOnlyFormats.length ? videoOnlyFormats : combinedFormats,
    audioFormats,
  };
};

/** A selected episode still needs playlist mode, even when only one is selected. */
export const getPlaylistDownloadOptions = (isPlaylist: boolean, selectedItems: number[]) => {
  if (!isPlaylist) return { noPlaylist: true, playlistItems: null };
  if (!selectedItems.length) return null;
  return {
    noPlaylist: false,
    playlistItems: [...new Set(selectedItems)].sort((a, b) => a - b).join(","),
  };
};
