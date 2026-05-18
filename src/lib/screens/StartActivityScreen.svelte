<script lang="ts">
  import { browser } from "$app/environment";
  import { isTauri } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { Button } from "bits-ui";
  import {
    getActivityTypes,
    getHeartRateStreamStatus,
    refreshUnfinishedActivityMetrics,
    startActivity,
    startHeartRateStream,
    stopHeartRateStream,
    updateActivity,
  } from "$lib/api";
  import type {
    ActivityHeartRateStatsSummary,
    ActivityTypeOption,
    HeartRateStreamStatus,
    UnfinishedActivitySummary,
  } from "$lib/api/interfaces";
  import type { SelectedWhoop } from "$lib/stores/selectedWhoop";
  import { onDestroy, onMount } from "svelte";

  export let whoop: SelectedWhoop;
  export let unfinishedActivity: UnfinishedActivitySummary | null = null;
  export let onBack: () => void = () => undefined;
  export let onStarted: () => void = () => undefined;
  export let onFinished: () => Promise<void> | void = () => undefined;

  type ActivityFilter = "all" | "strain" | "recovery" | "sleep";

  const filterTabs: { value: ActivityFilter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "strain", label: "Strain" },
    { value: "recovery", label: "Recovery" },
    { value: "sleep", label: "Sleep" },
  ];

  const heartRateEventName = "heart-rate-stream-sample";
  const staleHeartRateThresholdMs = 6500;
  const streamStatusPollIntervalMs = 1500;
  const strainRefreshIntervalMs = 15000;
  const zoneBoundaries = [0, 60, 90, 110, 130, 150, 180];

  interface HeartRateSampleEvent {
    bpm: number;
    receivedAtMs: number;
  }

  let activityTypeOptions: ActivityTypeOption[] = [];
  let selectedActivity = "";
  let selectedFilter: ActivityFilter = "all";
  let showActivityPicker = false;
  let loadingActivityTypes = false;
  let startingActivity = false;
  let finishingActivity = false;
  let isPaused = false;
  let nowMs = Date.now();
  let liveUnfinishedActivity: UnfinishedActivitySummary | null = null;
  let liveHeartRateStats: ActivityHeartRateStatsSummary | null = null;
  let currentBpm: number | null = null;
  let hrStreamRunning = false;
  let hrStreamBusy = false;
  let hrLastSignalAtMs: number | null = null;
  let hrSampleListener: UnlistenFn | null = null;
  let hrStatusTimer: ReturnType<typeof setInterval> | null = null;
  let strainRefreshTimer: ReturnType<typeof setInterval> | null = null;
  let exerciseServicesActive = false;
  let exerciseServicesBusy = false;
  let pauseStartedAtMs: number | null = null;
  let totalPausedMs = 0;
  let timerHandle: ReturnType<typeof setInterval> | null = null;

  $: hasUnfinishedActivity = unfinishedActivity !== null;
  $: activeUnfinishedActivity = liveUnfinishedActivity ?? unfinishedActivity;
  $: sortedActivityOptions = [...activityTypeOptions].sort((left, right) =>
    left.label.localeCompare(right.label),
  );
  $: filteredActivityOptions = sortedActivityOptions.filter((option) =>
    selectedFilter === "all"
      ? true
      : matchesFilter(option.label, selectedFilter),
  );
  $: selectedActivityText =
    activityTypeOptions.find((option) => option.value === selectedActivity)
      ?.label ?? "Select activity";
  $: activityStartMs = activeUnfinishedActivity
    ? new Date(activeUnfinishedActivity.start).getTime()
    : null;
  $: activePausedDurationMs =
    pauseStartedAtMs !== null ? Math.max(0, nowMs - pauseStartedAtMs) : 0;
  $: elapsedMs =
    activityStartMs === null
      ? 0
      : Math.max(
          0,
          nowMs - activityStartMs - totalPausedMs - activePausedDurationMs,
        );
  $: elapsedLabel = formatElapsedTime(elapsedMs);
  $: calibrationSecondsRemaining = Math.max(
    0,
    7 - Math.floor(elapsedMs / 1000),
  );
  $: calibrationReadyLabel =
    calibrationSecondsRemaining > 0
      ? `Ready in ${calibrationSecondsRemaining} ...`
      : "Ready";
  $: strainValue = activeUnfinishedActivity?.strain ?? null;
  $: heartRateValue = hasFreshHeartRateSignal() ? currentBpm : null;
  $: averageHeartRateValue = liveHeartRateStats?.avgHr ?? null;
  $: maxHeartRateValue = liveHeartRateStats?.maxHr ?? null;
  $: currentZoneIndex = heartRateZoneIndex(heartRateValue);
  $: zoneIndicatorPosition = heartRateZoneProgress(heartRateValue);
  $: caloriesValue = estimateCalories(elapsedMs, averageHeartRateValue);

  $: if (unfinishedActivity !== liveUnfinishedActivity) {
    liveUnfinishedActivity = unfinishedActivity;
  }

  $: if (activeUnfinishedActivity?.activity) {
    selectedActivity = activeUnfinishedActivity.activity;
  }

  $: if (!hasUnfinishedActivity) {
    isPaused = false;
    pauseStartedAtMs = null;
    totalPausedMs = 0;
    liveHeartRateStats = null;
  }

  onMount(() => {
    nowMs = Date.now();
    startTimer();
    void loadActivityTypes();
    if (hasUnfinishedActivity) {
      void startExerciseServices();
    }
  });

  onDestroy(() => {
    stopTimer();
    void stopExerciseServices();
  });

  $: if (browser && isTauri()) {
    if (hasUnfinishedActivity) {
      void startExerciseServices();
    } else if (exerciseServicesActive || exerciseServicesBusy) {
      void stopExerciseServices();
    }
  }

  function startTimer() {
    stopTimer();
    timerHandle = setInterval(() => {
      nowMs = Date.now();
    }, 1000);
  }

  function stopTimer() {
    if (timerHandle !== null) {
      clearInterval(timerHandle);
      timerHandle = null;
    }
  }

  function hasFreshHeartRateSignal(referenceTime = Date.now()) {
    return (
      hrLastSignalAtMs !== null &&
      referenceTime - hrLastSignalAtMs < staleHeartRateThresholdMs
    );
  }

  function commitHeartRateSample(bpm: number, receivedAtMs: number) {
    currentBpm = bpm;
    hrLastSignalAtMs = receivedAtMs;
    hrStreamRunning = true;
  }

  function applyHeartRateStreamStatus(status: HeartRateStreamStatus) {
    hrStreamRunning = status.running;

    if (
      status.running &&
      status.lastBpm !== null &&
      status.lastSampleAtMs !== null
    ) {
      commitHeartRateSample(status.lastBpm, status.lastSampleAtMs);
      return;
    }

    if (!status.running && !hasFreshHeartRateSignal()) {
      currentBpm = null;
      hrLastSignalAtMs = null;
    }
  }

  function heartRateZoneIndex(bpm: number | null) {
    if (bpm === null) {
      return 0;
    }

    for (let index = 0; index < zoneBoundaries.length - 2; index += 1) {
      if (bpm < zoneBoundaries[index + 1]) {
        return index;
      }
    }

    return 5;
  }

  function heartRateZoneProgress(bpm: number | null) {
    if (bpm === null) {
      return 1 / 12;
    }

    const clampedBpm = Math.max(
      zoneBoundaries[0],
      Math.min(bpm, zoneBoundaries[zoneBoundaries.length - 1]),
    );

    for (let index = 0; index < zoneBoundaries.length - 1; index += 1) {
      const start = zoneBoundaries[index];
      const end = zoneBoundaries[index + 1];

      if (clampedBpm <= end || index === zoneBoundaries.length - 2) {
        const span = Math.max(1, end - start);
        const progressWithinZone = (clampedBpm - start) / span;
        return (index + progressWithinZone) / 6;
      }
    }

    return 11 / 12;
  }

  function estimateCalories(durationMs: number, avgBpm: number | null) {
    if (avgBpm === null) {
      return 0;
    }

    const minutes = durationMs / 60000;
    const bpmLoad = Math.max(0, avgBpm - 70);
    return Math.max(0, Math.round(minutes * (0.8 + bpmLoad / 45)));
  }

  async function refreshLiveActivityMetrics() {
    if (!browser || !isTauri() || !hasUnfinishedActivity) {
      return;
    }

    const refreshedMetrics = await refreshUnfinishedActivityMetrics();
    liveUnfinishedActivity = refreshedMetrics.activity;
    liveHeartRateStats = refreshedMetrics.heartRateStats;
  }

  async function startExerciseServices() {
    if (
      !browser ||
      !isTauri() ||
      exerciseServicesActive ||
      exerciseServicesBusy ||
      !hasUnfinishedActivity
    ) {
      return;
    }

    exerciseServicesBusy = true;

    try {
      hrSampleListener = await listen<HeartRateSampleEvent>(
        heartRateEventName,
        (event) => {
          commitHeartRateSample(event.payload.bpm, event.payload.receivedAtMs);
        },
      );

      applyHeartRateStreamStatus(await getHeartRateStreamStatus());

      if (!hrStreamRunning || !hasFreshHeartRateSignal()) {
        hrStreamBusy = true;
        applyHeartRateStreamStatus(await startHeartRateStream());
        hrStreamBusy = false;
      }

      await refreshLiveActivityMetrics();

      hrStatusTimer = setInterval(async () => {
        if (!hasUnfinishedActivity || hrStreamBusy) {
          return;
        }

        try {
          applyHeartRateStreamStatus(await getHeartRateStreamStatus());
        } catch {
          currentBpm = hasFreshHeartRateSignal() ? currentBpm : null;
        }
      }, streamStatusPollIntervalMs);

      strainRefreshTimer = setInterval(() => {
        void refreshLiveActivityMetrics();
      }, strainRefreshIntervalMs);

      exerciseServicesActive = true;
    } finally {
      exerciseServicesBusy = false;
      hrStreamBusy = false;
    }
  }

  async function stopExerciseServices() {
    if (!browser || !isTauri()) {
      exerciseServicesActive = false;
      return;
    }

    if (hrStatusTimer !== null) {
      clearInterval(hrStatusTimer);
      hrStatusTimer = null;
    }

    if (strainRefreshTimer !== null) {
      clearInterval(strainRefreshTimer);
      strainRefreshTimer = null;
    }

    if (hrSampleListener) {
      hrSampleListener();
      hrSampleListener = null;
    }

    if (exerciseServicesActive) {
      try {
        await stopHeartRateStream();
      } catch {
        // Best effort: the UI can still fall back to the disconnected state.
      }
    }

    exerciseServicesActive = false;
    hrStreamRunning = false;
    hrStreamBusy = false;
    currentBpm = null;
    hrLastSignalAtMs = null;
    liveHeartRateStats = null;
  }

  async function loadActivityTypes() {
    loadingActivityTypes = true;

    try {
      activityTypeOptions = await getActivityTypes();
      selectedActivity = activityTypeOptions[0]?.value ?? "";
    } finally {
      loadingActivityTypes = false;
    }
  }

  function matchesFilter(label: string, filter: ActivityFilter): boolean {
    const normalized = label.trim().toLowerCase();

    if (filter === "sleep") {
      return normalized.includes("sleep") || normalized.includes("nap");
    }

    if (filter === "recovery") {
      return (
        normalized.includes("recovery") ||
        normalized.includes("massage") ||
        normalized.includes("stretch") ||
        normalized.includes("acupuncture") ||
        normalized.includes("compression") ||
        normalized.includes("meditation") ||
        normalized.includes("sauna") ||
        normalized.includes("cold") ||
        normalized.includes("bath")
      );
    }

    if (filter === "strain") {
      return (
        !matchesFilter(label, "sleep") && !matchesFilter(label, "recovery")
      );
    }

    return true;
  }

  function openActivityPicker() {
    if (loadingActivityTypes || startingActivity || hasUnfinishedActivity) {
      return;
    }

    showActivityPicker = true;
  }

  function closeActivityPicker() {
    showActivityPicker = false;
  }

  function selectActivity(option: ActivityTypeOption) {
    selectedActivity = option.value;
    closeActivityPicker();
  }

  function formatLocalDateTime(value: Date) {
    const year = value.getFullYear();
    const month = String(value.getMonth() + 1).padStart(2, "0");
    const day = String(value.getDate()).padStart(2, "0");
    const hours = String(value.getHours()).padStart(2, "0");
    const minutes = String(value.getMinutes()).padStart(2, "0");
    const seconds = String(value.getSeconds()).padStart(2, "0");
    return `${year}-${month}-${day}T${hours}:${minutes}:${seconds}`;
  }

  function formatElapsedTime(value: number) {
    const totalSeconds = Math.floor(value / 1000);
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;

    if (hours > 0) {
      return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
    }

    return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  function formatMetricValue(value: number | null) {
    return value === null ? "--" : `${Math.round(value)}`;
  }

  function togglePause() {
    if (!hasUnfinishedActivity) {
      return;
    }

    nowMs = Date.now();

    if (isPaused) {
      totalPausedMs +=
        pauseStartedAtMs === null ? 0 : Math.max(0, nowMs - pauseStartedAtMs);
      pauseStartedAtMs = null;
      isPaused = false;
      return;
    }

    pauseStartedAtMs = nowMs;
    isPaused = true;
  }

  async function handleFinishActivity() {
    if (
      !activeUnfinishedActivity ||
      activityStartMs === null ||
      finishingActivity
    ) {
      return;
    }

    finishingActivity = true;

    try {
      const finishAtMs = Math.max(Date.now(), activityStartMs + 1000);
      const finishAt = formatLocalDateTime(new Date(finishAtMs));

      await updateActivity(
        activeUnfinishedActivity.start,
        activeUnfinishedActivity.start,
        finishAt,
        activeUnfinishedActivity.activity,
      );
      await onFinished();
      onBack();
    } finally {
      finishingActivity = false;
    }
  }

  async function handleStartActivity() {
    if (!selectedActivity || loadingActivityTypes || startingActivity) {
      return;
    }

    startingActivity = true;

    try {
      await startActivity(formatLocalDateTime(new Date()), selectedActivity);
      onStarted();
    } finally {
      startingActivity = false;
    }
  }
</script>

<section class="start-activity-screen" aria-labelledby="start-activity-title">
  {#if hasUnfinishedActivity}
    <section class="exercise-screen" aria-label="Exercise screen">
      {#if isPaused}
        <header class="exercise-paused-topbar">
          <Button.Root
            class="exercise-paused-action"
            type="button"
            onclick={togglePause}
            disabled={finishingActivity}
          >
            <span class="exercise-paused-icon" aria-hidden="true">▶</span>
            <span>Resume</span>
          </Button.Root>

          <Button.Root
            class="exercise-paused-action exercise-paused-action--finish"
            type="button"
            onclick={() => void handleFinishActivity()}
            disabled={finishingActivity}
          >
            <span class="exercise-paused-icon" aria-hidden="true">⚑</span>
            <span>{finishingActivity ? "Saving" : "Finish"}</span>
          </Button.Root>
        </header>
      {:else}
        <header class="exercise-topbar">
          <Button.Root
            class="exercise-pause-button"
            type="button"
            aria-label="Pause activity"
            onclick={togglePause}
            disabled={finishingActivity}
          >
            <span></span>
            <span></span>
          </Button.Root>
          <p class="exercise-timer" aria-live="polite">{elapsedLabel}</p>
        </header>
      {/if}

      <div class="exercise-body">
        <div class="exercise-ring-shell" aria-hidden="true">
          <div class="exercise-ring">
            <svg viewBox="0 0 240 240">
              <circle class="exercise-ring-track" cx="120" cy="120" r="92"
              ></circle>
            </svg>

            <div class="exercise-ring-copy">
              <strong
                >{strainValue === null ? "--" : strainValue.toFixed(1)}</strong
              >
              {#if strainValue === null}
                <span>{calibrationReadyLabel}</span>
              {/if}
            </div>
          </div>
        </div>

        <div class="exercise-bottom">
          <section class="exercise-metric-block">
            <div class="exercise-metric-label">
              <span class="exercise-heart-icon" aria-hidden="true">♥</span>
              <span>Heart Rate</span>
            </div>
            <p class="exercise-primary-metric">
              {formatMetricValue(heartRateValue)}
            </p>
          </section>

          <section class="exercise-zone-block" aria-label="Heart rate zones">
            <div
              class="exercise-zone-bar"
              style={`--zone-indicator-left: calc((${zoneIndicatorPosition} * 100%) - 0.18rem);`}
            >
              <span class:active={currentZoneIndex === 0} class="zone zone-0"
              ></span>
              <span class:active={currentZoneIndex === 1} class="zone zone-1"
              ></span>
              <span class:active={currentZoneIndex === 2} class="zone zone-2"
              ></span>
              <span class:active={currentZoneIndex === 3} class="zone zone-3"
              ></span>
              <span class:active={currentZoneIndex === 4} class="zone zone-4"
              ></span>
              <span class:active={currentZoneIndex === 5} class="zone zone-5"
              ></span>
            </div>

            <div class="exercise-zone-labels">
              <span
                class:active={currentZoneIndex === 0}
                class="zone-label zone-label-0">Zone 0</span
              >
              <span
                class:active={currentZoneIndex === 1}
                class="zone-label zone-label-1">Zone 1</span
              >
              <span
                class:active={currentZoneIndex === 2}
                class="zone-label zone-label-2">Zone 2</span
              >
              <span
                class:active={currentZoneIndex === 3}
                class="zone-label zone-label-3">Zone 3</span
              >
              <span
                class:active={currentZoneIndex === 4}
                class="zone-label zone-label-4">Zone 4</span
              >
              <span
                class:active={currentZoneIndex === 5}
                class="zone-label zone-label-5">Zone 5</span
              >
            </div>
          </section>

          <section class="exercise-summary-grid" aria-label="Exercise summary">
            <article class="exercise-summary-card">
              <div class="exercise-summary-label">
                <span class="exercise-heart-icon" aria-hidden="true">♥</span>
                <span>Avg HR</span>
              </div>
              <strong>{formatMetricValue(averageHeartRateValue)}</strong>
            </article>

            <article class="exercise-summary-card">
              <div class="exercise-summary-label">
                <span aria-hidden="true">♥↑</span>
                <span>Max HR</span>
              </div>
              <strong>{formatMetricValue(maxHeartRateValue)}</strong>
            </article>

            <article class="exercise-summary-card">
              <div class="exercise-summary-label">
                <span aria-hidden="true">◔</span>
                <span>Calories</span>
              </div>
              <strong>{caloriesValue}</strong>
            </article>
          </section>
        </div>
      </div>
    </section>
  {:else}
    <div class="start-activity-shell">
      <header class="start-activity-topbar">
        <Button.Root
          class="start-activity-close"
          type="button"
          aria-label="Back"
          onclick={onBack}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </Button.Root>

        <Button.Root
          class="start-activity-picker-trigger"
          type="button"
          aria-haspopup="dialog"
          aria-expanded={showActivityPicker}
          aria-label="Select activity"
          disabled={loadingActivityTypes ||
            startingActivity ||
            hasUnfinishedActivity}
          onclick={openActivityPicker}
        >
          <span class="activity-glyph" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <circle cx="14.5" cy="4.6" r="2.2" />
              <path
                d="M12.2 8.2 9.6 11l2.8 2.6 1 4.8M11.9 8.4l4.4 1.1 1.9 3.3M9.8 11l-3.5 1.8M15 10.5l-2.1 4.1 5 1.3M15.7 18.6h-5.1"
              />
            </svg>
          </span>
          <span id="start-activity-title" class="start-activity-picker-label">
            {loadingActivityTypes ? "Loading..." : selectedActivityText}
          </span>
          <span
            class:open={showActivityPicker}
            class="start-activity-chevron"
            aria-hidden="true"
          >
            <svg viewBox="0 0 16 16">
              <path d="m3 6 5 5 5-5" />
            </svg>
          </span>
        </Button.Root>
      </header>

      <div class="start-activity-spacer"></div>

      <footer class="start-activity-footer">
        <p class="whoop-status">
          {whoop.connected ? "Device ready" : "Device disconnected"}
        </p>

        <Button.Root
          class="start-activity-button"
          type="button"
          disabled={!selectedActivity || loadingActivityTypes || startingActivity}
          onclick={() => void handleStartActivity()}
        >
          {startingActivity ? "Starting..." : "Start Activity"}
        </Button.Root>
      </footer>
    </div>

    {#if showActivityPicker}
      <div
        class="activity-picker-overlay"
        role="button"
        tabindex="0"
        aria-label="Close activity picker"
        on:click={closeActivityPicker}
        on:keydown={(event) => {
          if (
            event.key === "Escape" ||
            event.key === "Enter" ||
            event.key === " "
          ) {
            event.preventDefault();
            closeActivityPicker();
          }
        }}
      >
        <div
          class="activity-picker-panel"
          role="dialog"
          aria-modal="true"
          aria-labelledby="activity-picker-title"
          tabindex="-1"
          on:click|stopPropagation
          on:keydown|stopPropagation
        >
          <header class="activity-picker-topbar">
            <Button.Root
              class="start-activity-close"
              type="button"
              aria-label="Close activity picker"
              onclick={closeActivityPicker}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M6 6l12 12M18 6 6 18" />
              </svg>
            </Button.Root>

            <div class="activity-picker-heading">
              <span class="activity-glyph" aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  <circle cx="14.5" cy="4.6" r="2.2" />
                  <path
                    d="M12.2 8.2 9.6 11l2.8 2.6 1 4.8M11.9 8.4l4.4 1.1 1.9 3.3M9.8 11l-3.5 1.8M15 10.5l-2.1 4.1 5 1.3M15.7 18.6h-5.1"
                  />
                </svg>
              </span>
              <h2 id="activity-picker-title">{selectedActivityText}</h2>
              <span class="start-activity-chevron open" aria-hidden="true">
                <svg viewBox="0 0 16 16">
                  <path d="m3 6 5 5 5-5" />
                </svg>
              </span>
            </div>
          </header>

          <nav class="activity-filter-tabs" aria-label="Activity categories">
            {#each filterTabs as tab}
              <Button.Root
                class={`activity-filter-tab ${selectedFilter === tab.value ? "active" : ""}`}
                type="button"
                onclick={() => (selectedFilter = tab.value)}
              >
                {tab.label}
              </Button.Root>
            {/each}
          </nav>

          <div class="activity-picker-body">
            <p class="activity-picker-meta">
              {selectedFilter === "all"
                ? "All A - Z"
                : `${filterTabs.find((tab) => tab.value === selectedFilter)?.label ?? "All"} A - Z`}
            </p>

            <div class="activity-picker-list" aria-label="Activity options">
              {#if loadingActivityTypes}
                <p class="activity-picker-placeholder">
                  Loading activity types...
                </p>
              {:else if filteredActivityOptions.length > 0}
                {#each filteredActivityOptions as option}
                  <Button.Root
                    class={`activity-picker-option ${selectedActivity === option.value ? "active" : ""}`}
                    type="button"
                    onclick={() => selectActivity(option)}
                  >
                    <span
                      class="activity-picker-option-icon"
                      aria-hidden="true"
                    >
                      <svg viewBox="0 0 24 24">
                        <circle cx="14.5" cy="4.6" r="2.2" />
                        <path
                          d="M12.2 8.2 9.6 11l2.8 2.6 1 4.8M11.9 8.4l4.4 1.1 1.9 3.3M9.8 11l-3.5 1.8M15 10.5l-2.1 4.1 5 1.3M15.7 18.6h-5.1"
                        />
                      </svg>
                    </span>
                    <span>{option.label}</span>
                  </Button.Root>
                {/each}
              {:else}
                <p class="activity-picker-placeholder">
                  No matching activity types.
                </p>
              {/if}
            </div>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</section>

<style>
  .start-activity-screen,
  .activity-picker-panel,
  .exercise-screen {
    min-height: 100vh;
    min-height: 100dvh;
    color: #f4f8fb;
  }

  .start-activity-screen,
  .activity-picker-panel {
    background: radial-gradient(
        circle at top left,
        rgba(26, 111, 164, 0.22),
        transparent 28%
      ),
      linear-gradient(110deg, #092233 0%, #071826 56%, #04121c 100%);
  }

  .start-activity-screen {
    display: flex;
    flex-direction: column;
  }

  .exercise-screen {
    background: linear-gradient(180deg, #1b2529 0%, #1a2327 100%);
  }

  :global(button) {
    cursor: pointer;
  }

  :global(button:disabled) {
    cursor: progress;
    opacity: 0.72;
  }

  .exercise-topbar,
  .exercise-paused-topbar {
    min-height: max(2.55rem, calc(env(safe-area-inset-top, 0px) + 2.55rem));
    background: #1f97d6;
    color: #fff;
  }

  .exercise-topbar {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: env(safe-area-inset-top, 0px) 0.85rem 0;
  }

  :global(.exercise-pause-button) {
    width: 2rem;
    height: 2rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.18rem;
    padding: 0;
    border: 0;
    background: transparent;
    justify-self: center;
  }

  :global(.exercise-pause-button span) {
    width: 0.24rem;
    height: 0.72rem;
    border-radius: 999px;
    background: currentColor;
  }

  .exercise-timer {
    margin: 0;
    justify-self: center;
    font-size: 1.05rem;
    font-weight: 800;
    letter-spacing: 0.03em;
  }

  .exercise-paused-topbar {
    display: grid;
    grid-template-columns: 1fr 1fr;
    align-items: center;
    padding-top: env(safe-area-inset-top, 0px);
  }

  :global(.exercise-paused-action) {
    min-height: 2.55rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
    border: 0;
    background: transparent;
    color: #fff;
    font: inherit;
    font-size: 1.1rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  :global(.exercise-paused-action--finish) {
    border-left: 1px solid rgba(0, 0, 0, 0.12);
  }

  .exercise-paused-icon {
    font-size: 1.45rem;
    line-height: 1;
  }

  .exercise-body {
    min-height: calc(100vh - max(2.55rem, calc(env(safe-area-inset-top, 0px) + 2.55rem)));
    min-height: calc(100dvh - max(2.55rem, calc(env(safe-area-inset-top, 0px) + 2.55rem)));
    display: flex;
    flex-direction: column;
    padding: 0 0.75rem 1.4rem;
  }

  .exercise-ring-shell {
    display: flex;
    justify-content: center;
    padding: 3.1rem 0 2.4rem;
  }

  .exercise-bottom {
    margin-top: auto;
    padding-bottom: 0.5rem;
  }

  .exercise-ring {
    width: min(15rem, 76vw);
    aspect-ratio: 1;
    position: relative;
  }

  .exercise-ring svg {
    width: 100%;
    height: 100%;
  }

  .exercise-ring-track {
    fill: none;
    stroke: rgba(0, 0, 0, 0.52);
    stroke-width: 14;
  }

  .exercise-ring-copy {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 0.18rem;
    color: rgba(255, 255, 255, 0.86);
    text-align: center;
    text-transform: uppercase;
  }

  .exercise-ring-copy strong,
  .exercise-ring-copy span {
    margin: 0;
  }

  .exercise-ring-copy span {
    font-size: 0.72rem;
    font-weight: 800;
    letter-spacing: 0.06em;
  }

  .exercise-ring-copy strong {
    font-size: 2.5rem;
    line-height: 1;
    letter-spacing: 0.02em;
  }

  .exercise-metric-block {
    padding: 0 0 1.1rem;
  }

  .exercise-metric-label,
  .exercise-summary-label {
    display: grid;
    gap: 0.2rem;
    color: rgba(255, 255, 255, 0.28);
    font-size: 0.78rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .exercise-heart-icon {
    font-size: 1.05rem;
    line-height: 1;
  }

  .exercise-primary-metric {
    margin: 0.75rem 0 0;
    color: #fff;
    font-size: 2.05rem;
    font-weight: 700;
  }

  .exercise-zone-block {
    padding-bottom: 1.9rem;
  }

  .exercise-zone-bar {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 0.15rem;
    align-items: center;
    position: relative;
    margin-bottom: 0.55rem;
  }

  .zone {
    height: 0.34rem;
    border-radius: 999px;
    opacity: 0.36;
    transition:
      opacity 120ms ease,
      transform 120ms ease;
  }

  .zone-0 {
    background: #fff;
    opacity: 0.92;
  }

  .zone-1 {
    background: #758089;
  }

  .zone-2 {
    background: #294a59;
  }

  .zone-3 {
    background: #355c55;
  }

  .zone-4 {
    background: #675340;
  }

  .zone-5 {
    background: #6d402e;
  }

  .zone.active::after {
    content: "";
    position: absolute;
    left: var(--zone-indicator-left);
    top: 50%;
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: #fff;
    transform: translateY(-50%);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.16);
  }

  .zone.active {
    opacity: 0.95;
  }

  .exercise-zone-labels {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 0.2rem;
    font-size: 0.73rem;
    font-weight: 700;
  }

  .zone-label {
    text-align: center;
    opacity: 0.7;
    transition:
      opacity 120ms ease,
      color 120ms ease;
  }

  .zone-label.active {
    opacity: 0.95;
    font-weight: 800;
  }

  .zone-label-0 {
    color: #fff;
    opacity: 0.92;
  }

  .zone-label-1 {
    color: #7e8a93;
  }

  .zone-label-2 {
    color: #31586d;
  }

  .zone-label-3 {
    color: #446c62;
  }

  .zone-label-4 {
    color: #7b654f;
  }

  .zone-label-5 {
    color: #7b4937;
  }

  .exercise-summary-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    margin-top: auto;
  }

  .exercise-summary-card {
    padding: 0 0.55rem 0;
  }

  .exercise-summary-card + .exercise-summary-card {
    border-left: 1px solid rgba(255, 255, 255, 0.1);
  }

  .exercise-summary-card strong {
    display: block;
    margin-top: 0.85rem;
    color: #fff;
    font-size: 1.95rem;
    font-weight: 700;
  }

  .start-activity-topbar,
  .activity-picker-topbar {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    min-height: calc(env(safe-area-inset-top, 0px) + 4.1rem);
    padding: calc(env(safe-area-inset-top, 0px) + 0.9rem) 1.05rem 0.85rem;
    border-bottom: 1px solid rgba(34, 117, 168, 0.32);
  }

  :global(.start-activity-close),
  :global(.start-activity-button),
  :global(.start-activity-picker-trigger),
  :global(.activity-filter-tab),
  :global(.activity-picker-option) {
    border: 0;
    font: inherit;
  }

  :global(.start-activity-close) {
    width: 2rem;
    height: 2rem;
    display: grid;
    place-items: center;
    background: transparent;
    color: #f4f8fb;
    padding: 0;
    flex: 0 0 auto;
  }

  :global(.start-activity-close svg),
  .start-activity-chevron svg,
  .activity-glyph svg,
  .activity-picker-option-icon svg {
    width: 100%;
    height: 100%;
  }

  :global(.start-activity-close svg),
  .start-activity-chevron path,
  .activity-glyph circle,
  .activity-glyph path,
  .activity-picker-option-icon circle,
  .activity-picker-option-icon path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.9;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  :global(.start-activity-picker-trigger),
  .activity-picker-heading {
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 0.65rem;
    background: transparent;
    color: #f4f8fb;
    font-size: 0.98rem;
    font-weight: 800;
    letter-spacing: 0.01em;
  }

  :global(.start-activity-picker-trigger) {
    padding: 0;
  }

  :global(.start-activity-picker-trigger:disabled) {
    opacity: 0.6;
  }

  .activity-glyph,
  .activity-picker-option-icon {
    width: 1.35rem;
    height: 1.35rem;
    flex: 0 0 auto;
    color: #f4f8fb;
  }

  .start-activity-picker-label,
  .activity-picker-heading h2 {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .activity-picker-heading h2 {
    margin: 0;
    font-size: 0.98rem;
    font-weight: 800;
  }

  .start-activity-chevron {
    width: 0.9rem;
    height: 0.9rem;
    flex: 0 0 auto;
    color: rgba(244, 248, 251, 0.92);
    transition: transform 160ms ease;
  }

  .start-activity-chevron.open {
    transform: rotate(180deg);
  }

  .start-activity-shell {
    display: flex;
    flex: 1;
    min-height: 0;
    flex-direction: column;
  }

  .start-activity-spacer {
    flex: 1;
    min-height: 8rem;
  }

  .whoop-status {
    margin: 0 0 1rem;
    text-align: center;
    color: rgba(244, 248, 251, 0.74);
    font-size: 0.9rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .start-activity-footer {
    margin-top: auto;
    padding: 1rem 1rem calc(env(safe-area-inset-bottom, 0px) + 1rem);
    border-radius: 1.6rem 1.6rem 0 0;
    background: linear-gradient(
      180deg,
      rgba(2, 10, 16, 0),
      rgba(2, 10, 16, 0.82) 22%,
      #020b12 100%
    );
  }

  :global(.start-activity-button) {
    width: 100%;
    min-height: 3.2rem;
    border-radius: 999px;
    background: #f5f8fa;
    color: #08111a;
    font-weight: 800;
  }

  :global(.start-activity-button:disabled) {
    background: rgba(245, 248, 250, 0.46);
    color: rgba(8, 17, 26, 0.75);
  }

  .activity-picker-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: rgba(2, 7, 11, 0.88);
  }

  .activity-picker-panel {
    width: 100%;
    height: 100%;
    display: grid;
    grid-template-rows: auto auto 1fr;
  }

  .activity-filter-tabs {
    display: flex;
    align-items: center;
    gap: 1.35rem;
    padding: 0 1.05rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }

  :global(.activity-filter-tab) {
    position: relative;
    min-height: 2.9rem;
    padding: 0;
    background: transparent;
    color: rgba(244, 248, 251, 0.58);
    font-size: 0.73rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }

  :global(.activity-filter-tab.active) {
    color: #f4f8fb;
  }

  :global(.activity-filter-tab.active::after) {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 2px;
    background: #f4f8fb;
  }

  .activity-picker-body {
    min-height: 0;
    padding: 0.9rem 0 0;
  }

  .activity-picker-meta {
    margin: 0;
    padding: 0 1.05rem 0.9rem;
    color: rgba(244, 248, 251, 0.74);
    font-size: 0.8rem;
    font-weight: 600;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }

  .activity-picker-list {
    min-height: 0;
    height: 100%;
    overflow-y: auto;
    padding: 0.45rem 0 1.4rem;
  }

  :global(.activity-picker-option) {
    width: 100%;
    min-height: 3.45rem;
    display: flex;
    align-items: center;
    gap: 0.9rem;
    padding: 0.35rem 1.05rem;
    background: transparent;
    color: #f4f8fb;
    font-size: 1rem;
    font-weight: 800;
    text-align: left;
    text-transform: uppercase;
    letter-spacing: 0.01em;
  }

  :global(.activity-picker-option.active) {
    background: rgba(255, 255, 255, 0.05);
  }

  .activity-picker-placeholder {
    margin: 0;
    padding: 1rem 1.05rem;
    color: rgba(244, 248, 251, 0.68);
    font-size: 0.9rem;
  }
</style>
