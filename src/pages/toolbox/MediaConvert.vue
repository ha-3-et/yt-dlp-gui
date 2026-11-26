<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { showErrorDialog } from "@/utils/format";
import { goToolList } from "@/utils/toolbox";
import { useStatusStore } from "@/stores/status";
import { useI18n } from "vue-i18n";

interface OutputFormat {
  format: string;
  extension: string;
}

interface ConversionResult {
  input: string;
  output: string | null;
  status: "completed" | "failed" | "skipped";
  error: string | null;
}

interface ConversionProgress {
  run_id: string;
  current: number;
  total: number;
  file: string;
  status: string;
  output: string | null;
  error: string | null;
}

const { t } = useI18n();
const router = useRouter();
const statusStore = useStatusStore();

const files = ref<string[]>([]);
const formats = ref<OutputFormat[]>([]);
const inputExtensions = ref<string[]>([]);
const sourceFormat = ref("all");
const outputFormat = ref("");
const outputDirectory = ref("");
const mode = ref<"copy" | "encode">("copy");
const loadingFormats = ref(false);
const scanning = ref(false);
const converting = ref(false);
const progress = ref<ConversionProgress | null>(null);
const results = ref<ConversionResult[]>([]);

const fileExtension = (path: string) => {
  const name = path.replace(/\\/g, "/").split("/").pop() || "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
};
const fileName = (path: string) => path.replace(/\\/g, "/").split("/").pop() || path;
const sourceExtensions = computed(() =>
  [...new Set(files.value.map(fileExtension).filter((ext) => inputExtensions.value.includes(ext)))].sort(),
);
const sourceFormatOptions = computed(() => [
  { label: t("toolbox.mediaAllFormats"), value: "all" },
  ...sourceExtensions.value.map((ext) => ({ label: ext.toUpperCase(), value: ext })),
]);
const filteredFiles = computed(() =>
  sourceFormat.value === "all"
    ? files.value
    : files.value.filter((path) => fileExtension(path) === sourceFormat.value),
);
const formatOptions = computed(() =>
  formats.value.map((item) => ({
    label:
      item.extension.toUpperCase() === item.format.toUpperCase()
        ? item.extension.toUpperCase()
        : `${item.extension.toUpperCase()} (${item.format})`,
    value: item.format,
  })),
);
const resultCounts = computed(() => ({
  completed: results.value.filter((item) => item.status === "completed").length,
  failed: results.value.filter((item) => item.status === "failed").length,
  skipped: results.value.filter((item) => item.status === "skipped").length,
}));

const loadFormats = async () => {
  loadingFormats.value = true;
  try {
    [formats.value, inputExtensions.value] = await Promise.all([
      invoke<OutputFormat[]>("tool_get_media_output_formats"),
      invoke<string[]>("tool_get_media_input_extensions"),
    ]);
    if (!outputFormat.value) {
      outputFormat.value = formats.value.some((item) => item.format === "mp4")
        ? "mp4"
        : (formats.value[0]?.format ?? "");
    }
  } catch (error) {
    showErrorDialog(String(error));
  } finally {
    loadingFormats.value = false;
  }
};

const prepareConverter = async () => {
  try {
    const status = await invoke<{ runnable: boolean }>("get_ffmpeg_status");
    if (!status.runnable) {
      statusStore.showFfmpegSetupModal = true;
      return;
    }
    await loadFormats();
  } catch (error) {
    showErrorDialog(String(error));
  }
};

const chooseFiles = async () => {
  const selected = await open({ multiple: true, title: t("toolbox.mediaChooseFiles") });
  if (!selected) return;
  const paths = Array.isArray(selected) ? selected : [selected];
  const supported = paths.filter((path) => inputExtensions.value.includes(fileExtension(path)));
  if (supported.length < paths.length) {
    window.$message.warning(
      t("toolbox.mediaUnsupportedFiles", { n: paths.length - supported.length }),
    );
  }
  files.value = [...new Set([...files.value, ...supported])];
  results.value = [];
};

const chooseDirectory = async () => {
  const selected = await open({
    directory: true,
    multiple: false,
    title: t("toolbox.mediaChooseSourceFolder"),
  });
  if (!selected || Array.isArray(selected)) return;
  scanning.value = true;
  try {
    const paths = await invoke<string[]>("tool_scan_media_directory", { directory: selected });
    const supported = paths.filter((path) => inputExtensions.value.includes(fileExtension(path)));
    if (supported.length < paths.length) {
      window.$message.info(
        t("toolbox.mediaUnsupportedFiles", { n: paths.length - supported.length }),
      );
    }
    files.value = [...new Set([...files.value, ...supported])];
    results.value = [];
  } catch (error) {
    showErrorDialog(String(error));
  } finally {
    scanning.value = false;
  }
};

const chooseOutputDirectory = async () => {
  const selected = await open({
    directory: true,
    multiple: false,
    title: t("toolbox.mediaChooseOutputFolder"),
  });
  if (selected && !Array.isArray(selected)) outputDirectory.value = selected;
};

const removeFile = (path: string) => {
  files.value = files.value.filter((item) => item !== path);
  if (sourceFormat.value !== "all" && !sourceExtensions.value.includes(sourceFormat.value)) {
    sourceFormat.value = "all";
  }
};

const clearFiles = () => {
  files.value = [];
  results.value = [];
  sourceFormat.value = "all";
};

const resultDescription = (item: ConversionResult) => {
  if (item.status === "completed") return item.output || "";
  if (item.error === "err_media_output_exists") return t("toolbox.mediaOutputExists");
  if (item.error === "err_media_same_input_output") return t("toolbox.mediaSameFormat");
  return item.error || item.output || "";
};

const startConversion = async () => {
  if (!filteredFiles.value.length || !outputFormat.value || converting.value) return;
  const runId = `media-convert-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  converting.value = true;
  progress.value = null;
  results.value = [];
  let unlisten = () => {};
  try {
    unlisten = await listen<ConversionProgress>("media-convert-progress", (event) => {
      if (event.payload.run_id === runId) progress.value = event.payload;
    });
    results.value = await invoke<ConversionResult[]>("tool_convert_media_files", {
      runId,
      files: filteredFiles.value,
      outputFormat: outputFormat.value,
      outputDirectory: outputDirectory.value || null,
      mode: mode.value,
    });
    if (resultCounts.value.failed === 0) {
      window.$message.success(
        t("toolbox.mediaConversionCompleted", {
          count: resultCounts.value.completed,
          skipped: resultCounts.value.skipped,
        }),
      );
    }
  } catch (error) {
    showErrorDialog(String(error));
  } finally {
    unlisten();
    converting.value = false;
  }
};

const handleBack = () => goToolList(router);

onMounted(() => {
  void prepareConverter();
});
</script>

<template>
  <n-flex vertical :size="12">
    <n-flex align="center" :size="8">
      <n-button strong secondary size="small" @click="handleBack">
        <template #icon><n-icon><icon-mdi-arrow-left /></n-icon></template>
        {{ $t("common.back") }}
      </n-button>
      <n-text strong style="font-size: 15px">{{ $t("toolbox.mediaConvertTitle") }}</n-text>
    </n-flex>

    <n-card size="small">
      <n-flex vertical :size="12">
        <n-text depth="3" style="font-size: 13px">
          {{ $t("toolbox.mediaConvertPageDesc") }}
        </n-text>
        <n-flex :size="8" wrap>
          <n-button size="small" :disabled="loadingFormats" @click="chooseFiles">
            <template #icon><n-icon><icon-mdi-file-video-outline /></n-icon></template>
            {{ $t("toolbox.mediaChooseFiles") }}
          </n-button>
          <n-button
            size="small"
            :loading="scanning"
            :disabled="loadingFormats"
            @click="chooseDirectory"
          >
            <template #icon><n-icon><icon-mdi-folder-multiple-outline /></n-icon></template>
            {{ $t("toolbox.mediaChooseSourceFolder") }}
          </n-button>
          <n-button size="small" :disabled="!files.length" @click="clearFiles">
            {{ $t("toolbox.mediaClearFiles") }}
          </n-button>
        </n-flex>

        <n-flex align="center" :size="8" wrap>
          <n-text depth="3">{{ $t("toolbox.mediaSourceFormat") }}</n-text>
          <n-select
            v-model:value="sourceFormat"
            :options="sourceFormatOptions"
            size="small"
            style="width: 140px"
          />
          <n-text depth="3">{{ $t("toolbox.mediaFileCount", { n: filteredFiles.length }) }}</n-text>
        </n-flex>

        <n-scrollbar style="max-height: 220px">
          <n-empty v-if="!filteredFiles.length" :description="$t('toolbox.mediaNoFiles')" />
          <n-list v-else bordered size="small">
            <n-list-item v-for="path in filteredFiles" :key="path">
              <n-thing :title="fileName(path)" :description="path" />
              <template #suffix>
                <n-button quaternary circle size="tiny" @click="removeFile(path)">
                  <template #icon><n-icon><icon-mdi-close /></n-icon></template>
                </n-button>
              </template>
            </n-list-item>
          </n-list>
        </n-scrollbar>
      </n-flex>
    </n-card>

    <n-card size="small" :title="$t('toolbox.mediaOutputSettings')">
      <n-flex vertical :size="12">
        <n-flex align="center" :size="8" wrap>
          <n-text depth="3">{{ $t("toolbox.mediaOutputFormat") }}</n-text>
          <n-select
            v-model:value="outputFormat"
            :options="formatOptions"
            :loading="loadingFormats"
            filterable
            size="small"
            style="width: 200px"
          />
        </n-flex>
        <n-flex align="center" :size="8" wrap>
          <n-text depth="3">{{ $t("toolbox.mediaConversionMode") }}</n-text>
          <n-radio-group v-model:value="mode" size="small">
            <n-radio-button value="copy">{{ $t("toolbox.mediaCopyMode") }}</n-radio-button>
            <n-radio-button value="encode">{{ $t("toolbox.mediaEncodeMode") }}</n-radio-button>
          </n-radio-group>
        </n-flex>
        <n-text depth="3" style="font-size: 12px">
          {{ mode === "copy" ? $t("toolbox.mediaCopyHint") : $t("toolbox.mediaEncodeHint") }}
        </n-text>
        <n-flex align="center" :size="8" wrap>
          <n-input
            v-model:value="outputDirectory"
            :placeholder="$t('toolbox.mediaOutputFolderDefault')"
            size="small"
            readonly
            style="flex: 1; min-width: 220px"
          />
          <n-button size="small" @click="chooseOutputDirectory">
            <template #icon><n-icon><icon-mdi-folder-open-outline /></n-icon></template>
            {{ $t("toolbox.mediaChooseOutputFolder") }}
          </n-button>
          <n-button v-if="outputDirectory" size="small" @click="outputDirectory = ''">
            {{ $t("toolbox.mediaOutputFolderDefaultButton") }}
          </n-button>
        </n-flex>
        <n-text depth="3" style="font-size: 12px">
          {{ $t("toolbox.mediaNoOverwriteHint") }}
        </n-text>
        <n-button
          type="primary"
          :loading="converting"
          :disabled="!filteredFiles.length || !outputFormat || converting"
          @click="startConversion"
        >
          <template #icon><n-icon><icon-mdi-movie-open-cog /></n-icon></template>
          {{ $t("toolbox.mediaStartConversion", { n: filteredFiles.length }) }}
        </n-button>
      </n-flex>
    </n-card>

    <n-card v-if="progress || results.length" size="small" :title="$t('toolbox.mediaProgress')">
      <n-flex vertical :size="8">
        <n-text v-if="progress" depth="3">
          {{
            $t("toolbox.mediaProcessing", {
              current: progress.current,
              total: progress.total,
              file: progress.file,
            })
          }}
        </n-text>
        <n-progress
          v-if="progress"
          type="line"
          :percentage="Math.round((progress.current / progress.total) * 100)"
          :status="progress.status === 'failed' ? 'error' : undefined"
          :show-indicator="true"
        />
        <n-alert
          v-if="results.length"
          :type="resultCounts.failed ? 'warning' : 'success'"
          :bordered="false"
        >
          {{ $t("toolbox.mediaResultSummary", resultCounts) }}
        </n-alert>
        <n-list v-if="results.length" bordered size="small">
          <n-list-item v-for="item in results" :key="item.input">
            <n-thing :title="fileName(item.input)" :description="resultDescription(item)">
              <template #header-extra>
                <n-tag
                  size="small"
                  :type="
                    item.status === 'completed'
                      ? 'success'
                      : item.status === 'failed'
                        ? 'error'
                        : 'warning'
                  "
                >
                  {{ $t(`toolbox.mediaStatus.${item.status}`) }}
                </n-tag>
              </template>
            </n-thing>
          </n-list-item>
        </n-list>
      </n-flex>
    </n-card>
  </n-flex>
</template>
