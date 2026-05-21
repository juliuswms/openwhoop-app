<script lang="ts">
  import { Button, Tabs } from "bits-ui";
  import type { SelectedWhoop } from "$lib/stores/selectedWhoop";
  import type { AlarmInfo } from "$lib/api/interfaces";
  import {
    batteryDetail,
    batteryPercentLabel,
    connectedDeviceLabel,
    connectionLabel,
    deviceIdLabel,
    hasSelectedWhoop,
    isDeviceManagementBusy,
    lastSyncLabel,
    statusHeadline,
  } from "$lib/stores/deviceManagement";

  type Tab = "status" | "advanced" | "alarms";

  export let whoop: SelectedWhoop;
  export let latestSyncLabel = "--:--";
  export let batteryLevel: number | null = null;
  export let error = "";
  export let clearing = false;
  export let reconnecting = false;
  export let rebooting = false;
  export let erasing = false;
  export let alarmBusy = false;
  export let alarmInfo: AlarmInfo | null = null;
  export let showReconnect = false;
  export let onReconnect: () => Promise<void> | void = () => undefined;
  export let onChooseAnother: () => Promise<void> | void = () => undefined;
  export let onReboot: () => Promise<void> | void = () => undefined;
  export let onErase: () => Promise<void> | void = () => undefined;
  export let onSetAlarm: (unix: number) => Promise<void> | void = () => undefined;
  export let onRingAlarm: () => Promise<void> | void = () => undefined;
  export let onGetAlarm: () => Promise<void> | void = () => undefined;
  export let onOpenScan: () => void = () => undefined;
  export let onBack: () => void = () => undefined;

  let activeTab: Tab = "status";
  let alarmHour = 7;
  let alarmMinute = 0;

  $: busy = isDeviceManagementBusy(reconnecting, clearing, rebooting, erasing);
  $: paired = hasSelectedWhoop(whoop);
  $: connectionTitle = paired
    ? whoop.connected
      ? "Connected to"
      : "Not connected to"
    : "No device paired";

  $: if (activeTab === "alarms" && onGetAlarm) {
    void onGetAlarm();
  }

  function alarmHoursOption() {
    const hours = [];
    for (let h = 0; h < 24; h++) {
      hours.push(String(h).padStart(2, "0"));
    }
    return hours;
  }

  function alarmMinutesOption() {
    const minutes = [];
    for (let m = 0; m < 60; m++) {
      minutes.push(String(m).padStart(2, "0"));
    }
    return minutes;
  }

  function handleSetAlarm() {
    const now = new Date();
    const alarmDate = new Date(now.getFullYear(), now.getMonth(), now.getDate(), alarmHour, alarmMinute, 0);
    if (alarmDate <= now) {
      alarmDate.setDate(alarmDate.getDate() + 1);
    }
    const unix = Math.floor(alarmDate.getTime() / 1000);
    void onSetAlarm(unix);
  }

  function alarmTimeLabel(info: AlarmInfo): string {
    if (!info.enabled) {
      return "No alarm set";
    }
    const date = new Date(info.unix * 1000);
    return `Set for ${date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  }
</script>

<section class="screen-shell" aria-labelledby="device-management-title">
  <div class="screen-stack">
    <header class="screen-header screen-header--balanced">
      <Button.Root
        class="ui-button ui-button--ghost"
        style="min-height: 0; width: 2.5rem; height: 2.5rem; padding: 0; justify-content: center;"
        type="button"
        aria-label="Close device settings"
        onclick={onBack}
      >
        <span aria-hidden="true">←</span>
      </Button.Root>

      <div class="screen-header__center">
        <h1 id="device-management-title">Device settings</h1>
      </div>

      <span aria-hidden="true" style="width: 2.5rem; height: 2.5rem; display: inline-block;"></span>
    </header>

    <Tabs.Root class="stack-sm" bind:value={activeTab}>
      <Tabs.List class="tab-list" aria-label="Device settings sections">
        <Tabs.Trigger class="tab-trigger" value="status">
          Status
        </Tabs.Trigger>
        <Tabs.Trigger class="tab-trigger" value="advanced">
          Advanced
        </Tabs.Trigger>
        <Tabs.Trigger class="tab-trigger" value="alarms">
          Alarms
        </Tabs.Trigger>
      </Tabs.List>

      <Tabs.Content value="status">
        <section class="panel stack-sm" aria-label="Device status">
          <div class="split-row split-row--top">
            <div>
              <p class="eyebrow">{connectionTitle}</p>
              <h2>{connectedDeviceLabel(whoop)}</h2>
            </div>

            <div class="summary-badge-column">
              <p class="eyebrow">Last sync</p>
              <strong>{lastSyncLabel(whoop, latestSyncLabel)}</strong>
            </div>
          </div>

          <div class="detail-grid">
            <article class="detail-card">
              <p class="detail-label">Status</p>
              <strong>{statusHeadline(whoop, reconnecting)}</strong>
              <p class="muted">{connectionLabel(whoop, reconnecting)}</p>
            </article>

            <article class="detail-card">
              <p class="detail-label">Battery level</p>
              <strong>{batteryPercentLabel(batteryLevel)}</strong>
              <p class="muted">{batteryDetail(whoop, batteryLevel)}</p>
            </article>
          </div>

          <div class="detail-card">
            <p class="detail-label">Device ID</p>
            <strong class="mono">{deviceIdLabel(whoop)}</strong>
          </div>

          {#if error}
            <p class="alert alert--error">{error}</p>
          {/if}

          {#if showReconnect && paired}
            <Button.Root
              class="ui-button ui-button--secondary ui-button--full"
              type="button"
              disabled={busy}
              onclick={() => void onReconnect()}
            >
              {reconnecting ? "Reconnecting..." : "Reconnect device"}
            </Button.Root>
          {:else if !paired}
            <Button.Root
              class="ui-button ui-button--secondary ui-button--full"
              type="button"
              disabled={busy}
              onclick={onOpenScan}
            >
              Pair a device
            </Button.Root>
          {/if}
        </section>
      </Tabs.Content>

      <Tabs.Content value="advanced">
        <section class="list-stack" aria-label="Advanced settings">
          <article class="panel stack-xs">
            <Button.Root
              class="ui-button ui-button--full"
              type="button"
              disabled={busy}
              onclick={onOpenScan}
            >
              Pair a device
            </Button.Root>
            <p class="muted">
              Pair another WHOOP to the app. This replaces the saved pairing.
            </p>
          </article>

          <article class="panel stack-xs">
            <Button.Root
              class="ui-button ui-button--full"
              type="button"
              disabled={!paired || busy}
              onclick={() => void onChooseAnother()}
            >
              {clearing ? "Unpairing..." : "Unpair device"}
            </Button.Root>
            <p class="muted">
              Unpair your WHOOP from the app. This removes the Bluetooth target.
            </p>
          </article>

          <article class="panel stack-xs">
            <Button.Root
              class="ui-button ui-button--full"
              type="button"
              disabled
            >
              Firmware check
            </Button.Root>
            <p class="muted">Check and install the latest WHOOP firmware.</p>
          </article>

          <article class="panel stack-xs">
            <Button.Root
              class="ui-button ui-button--danger ui-button--full"
              type="button"
              disabled={!paired || busy}
              onclick={() => void onErase()}
            >
              {erasing ? "Erasing..." : "Erase device data"}
            </Button.Root>
            <p class="muted">
              Erase all heart rate data currently stored on your WHOOP.
            </p>
          </article>

          <article class="panel stack-xs">
            <Button.Root
              class="ui-button ui-button--full"
              type="button"
              disabled={!paired || busy}
              onclick={() => void onReboot()}
            >
              {rebooting ? "Rebooting..." : "Reboot device"}
            </Button.Root>
            <p class="muted">Reboot your WHOOP to restart the device.</p>
          </article>
        </section>
      </Tabs.Content>

      <Tabs.Content value="alarms">
        <section class="list-stack" aria-label="Alarm settings">
          <article class="panel stack-xs">
            <p class="eyebrow">Current Alarm</p>
            {#if alarmInfo}
              <strong>{alarmTimeLabel(alarmInfo)}</strong>
            {:else}
              <strong class="muted">Not checked</strong>
            {/if}
            <Button.Root
              class="ui-button ui-button--secondary ui-button--full"
              style="margin-top: 0.5rem;"
              type="button"
              disabled={!whoop.connected || alarmBusy}
              onclick={() => void onGetAlarm()}
            >
              {alarmBusy ? "Checking..." : "Refresh alarm status"}
            </Button.Root>
          </article>

          <article class="panel stack-xs">
            <p class="eyebrow">Set Alarm Time</p>
            <div style="display: flex; gap: 0.5rem; align-items: center; margin-top: 0.5rem;">
              <select bind:value={alarmHour} class="ui-select" aria-label="Hour">
                {#each alarmHoursOption() as hour}
                  <option value={Number(hour)}>{hour}</option>
                {/each}
              </select>
              <span aria-hidden="true">:</span>
              <select bind:value={alarmMinute} class="ui-select" aria-label="Minute">
                {#each alarmMinutesOption() as minute}
                  <option value={Number(minute)}>{minute}</option>
                {/each}
              </select>
            </div>
            <Button.Root
              class="ui-button ui-button--full"
              style="margin-top: 0.5rem;"
              type="button"
              disabled={!whoop.connected || alarmBusy}
              onclick={handleSetAlarm}
            >
              {alarmBusy ? "Setting..." : "Set Alarm"}
            </Button.Root>
            <p class="muted">
              Sets a strap-driven alarm on your WHOOP. The device will vibrate at the configured time.
            </p>
          </article>

          <article class="panel stack-xs">
            <Button.Root
              class="ui-button ui-button--secondary ui-button--full"
              type="button"
              disabled={!whoop.connected || alarmBusy}
              onclick={() => void onRingAlarm()}
            >
              {alarmBusy ? "Ringing..." : "Ring Now"}
            </Button.Root>
            <p class="muted">
              Immediately triggers the vibration alarm on your WHOOP for testing.
            </p>
          </article>
        </section>
      </Tabs.Content>
    </Tabs.Root>
  </div>
</section>
