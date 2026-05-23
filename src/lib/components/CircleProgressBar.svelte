<script>
  export let progress;
  export let color;
  $: angle = 360 * progress;
  const thickness = 65;

  $: progressArc = `conic-gradient(
    ${color} 0deg ${angle}deg,
    transparent ${angle}deg 360deg
    )`;

  const maskGradient = `radial-gradient(circle, transparent ${thickness}%, black ${thickness}%)`;
  $: cssVarStyles = `
      --progress-arc: ${progressArc};
      --mask: ${maskGradient};
    `;
</script>

<style>
    #progress-circle {
      position: relative;
      border-radius: 50%;
      padding: 0.01rem;
      background: transparent;
    }

    /* The progress ring – behind the slot content */
    #progress-circle::before {
      content: '';
      position: absolute;
      top: 0;
      left: 0;
      width: 100%;
      height: 100%;
      border-radius: inherit;

      /* Draw the coloured arc */
      background: var(--progress-arc);

      /* Mask away the centre */
      -webkit-mask: var(--mask);
      mask: var(--mask);
    }
</style>

<div id="progress-circle" style={cssVarStyles}>
  <slot />
</div>
