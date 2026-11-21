// The comment UI's styles, inside its shadow root.
const dark = `
    color-scheme: dark;
    --jp-surface: #161616;
    --jp-surface-header: #24272d;
    --jp-field: #202124;
    --jp-hover: #303338;
    --jp-border: #474b52;
    --jp-input-border: #717780;
    --jp-text: #f5f6f8;
    --jp-muted: #b9c0cb;
    --jp-link: #9ac8ff;
    --jp-focus: #84b9ff;
    --jp-selected: #253d60;
    --jp-warning: #f0c27a;
    --jp-error: #ffa8a8;
    --jp-success: #8ce99a;
    --jp-shadow: 0 calc(8 * var(--jp-unit)) calc(24 * var(--jp-unit)) rgb(0 0 0 / 26%);
  `;

export const styles = `
  :host { all: initial; }

  #overlay-root {
    all: initial;
    display: contents;
    color-scheme: light;
    /* --jp-unit is set on this root by Ui from the validated text size. */
    --jp-font: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    font-family: var(--jp-font);
    --jp-surface: #fff;
    --jp-surface-header: #f0f3f7;
    --jp-field: #f6f7fa;
    --jp-hover: #e9edf4;
    --jp-border: #d5dbe5;
    --jp-input-border: #7b8494;
    --jp-text: #202632;
    --jp-muted: #535f71;
    --jp-link: #1456ad;
    --jp-focus: #246bcb;
    --jp-selected: #dceaff;
    --jp-warning: #784b11;
    --jp-error: #a22222;
    --jp-success: #176630;
    --jp-shadow: 0 calc(8 * var(--jp-unit)) calc(24 * var(--jp-unit)) rgb(28 40 60 / 16%);
    --jp-accent: #225fbe;
    --jp-on-accent: #fff;
    --jp-anchor: #2479e9;
    --jp-anchor-active: #1456ad;
    --jp-tint: rgb(47 128 237 / 8%);
    --jp-connected: #2f9e44;
    --jp-switch-on: #288337;
    --jp-reconnecting: #e8a33d;
    --jp-disconnected: #9a958e;
    --jp-cursor-text: #fff;
    --jp-radius: calc(11 * var(--jp-unit));
    font-size: calc(13 * var(--jp-unit));
  }

  :host([data-theme="dark"]) #overlay-root { ${dark} }
  @media (prefers-color-scheme: dark) {
    :host(:not([data-theme="light"])) #overlay-root { ${dark} }
  }

  *, *::before, *::after { box-sizing: border-box; }

  #highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid var(--jp-anchor);
    background: var(--jp-tint);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 90%);
    pointer-events: none;
  }

  #edit-highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid var(--jp-anchor);
    background: var(--jp-tint);
    pointer-events: none;
  }

  #bar {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(14 * var(--jp-unit));
    z-index: 2147483644;
    display: flex;
    align-items: center;
    gap: calc(6 * var(--jp-unit));
  }

  #bar[hidden] { display: none; }

  #pill, #gear, #edit, #pdf-outline, #attach { height: calc(32 * var(--jp-unit)); }

  #pill {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--jp-unit));
    padding: 0 calc(12 * var(--jp-unit)) 0 calc(9 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 999px;
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    cursor: pointer;
    font: 600 calc(12 * var(--jp-unit))/1 var(--jp-font);
  }

  .dot {
    display: block;
    flex: none;
    width: calc(10 * var(--jp-unit));
    height: calc(10 * var(--jp-unit));
    border-radius: 50%;
    background: var(--jp-disconnected);
  }

  .dot[data-state="green"] { background: var(--jp-connected); }
  .dot[data-state="amber"] { background: var(--jp-reconnecting); }
  .dot[data-state="grey"] { background: var(--jp-disconnected); }

  #gear, #edit, #pdf-outline, #attach {
    display: flex;
    align-items: center;
    justify-content: center;
    width: calc(32 * var(--jp-unit));
    padding: 0;
    border: 1px solid var(--jp-border);
    border-radius: 50%;
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    cursor: pointer;
  }

  :is(#pill, #gear, #edit, #pdf-outline, #attach):hover { background: var(--jp-hover); }
  #pdf-outline[hidden] { display: none; }
  #gear[aria-expanded="true"], #pdf-outline[aria-expanded="true"] { background: var(--jp-hover); }
  #edit[aria-pressed="true"] { border-color: var(--jp-accent); background: var(--jp-accent); color: var(--jp-on-accent); }

  #list {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(14 * var(--jp-unit));
    z-index: 2147483644;
    display: flex;
    flex-direction: column;
    width: min(calc(440 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit)));
    max-height: min(60vh, calc(100vh - 28 * var(--jp-unit)));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(12 * var(--jp-unit))/1.45 var(--jp-font);
    overflow: hidden;
  }

  #list[hidden] { display: none; }

  #list header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: calc(10 * var(--jp-unit));
    padding: calc(12 * var(--jp-unit)) calc(14 * var(--jp-unit));
    border-bottom: 1px solid var(--jp-border);
    cursor: pointer;
  }

  #list-summary {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--jp-unit));
    min-width: 0;
    font-weight: 600;
  }

  #connection-text {
    overflow: hidden;
    color: var(--jp-muted);
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  #connection-text[data-state="grey"], #connection-text[data-state="amber"] { color: var(--jp-warning); }

  #collapse {
    flex: none;
    width: calc(28 * var(--jp-unit));
    height: calc(28 * var(--jp-unit));
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--jp-muted);
    font: calc(16 * var(--jp-unit))/1 var(--jp-font);
  }

  #collapse:hover, #collapse:focus-visible { background: var(--jp-hover); }

  #list-content { overflow: auto; }

  #outbox-list, #answer-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: calc(4 * var(--jp-unit)) 0;
    list-style: none;
  }

  #outbox-list > li, #answer-list > li {
    display: flex;
    flex-direction: column;
    gap: calc(4 * var(--jp-unit));
    padding: calc(10 * var(--jp-unit)) calc(14 * var(--jp-unit));
    border-bottom: 1px solid var(--jp-border);
  }

  #outbox-list > li:last-child, #answer-list > li:last-child { border-bottom: 0; }
  #outbox-list:empty, #answer-list:empty { display: none; }
  #outbox-list:not(:empty) + #answer-list:not(:empty) { border-top: 1px solid var(--jp-border); }

  .agent-reply { min-width: 0; color: var(--jp-text); overflow-wrap: anywhere; }
  #outbox-list .agent-reply {
    margin-top: calc(4 * var(--jp-unit));
    padding-inline-start: calc(8 * var(--jp-unit));
    border-inline-start: 2px solid var(--jp-border);
  }
  .agent-reply p { margin: 0 0 calc(4 * var(--jp-unit)); }
  .agent-reply p:last-child { margin-bottom: 0; }
  .agent-reply a { color: var(--jp-link); }
  .agent-reply pre { overflow: auto; }
  .agent-reply table { display: block; overflow: auto; }
  #outbox-list .row { display: flex; justify-content: space-between; gap: calc(10 * var(--jp-unit)); }
  #outbox-list .where, #outbox-list .state { color: var(--jp-muted); }
  #outbox-list .where { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #outbox-list li[data-state="delivered"] .state { color: var(--jp-success); }
  .sending-indicator {
    display: inline-block;
    width: .65em; height: .65em;
    margin-right: .45em;
    border: 1px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: jamparty-sending .8s linear infinite;
  }
  @keyframes jamparty-sending { to { transform: rotate(1turn); } }
  @media (prefers-reduced-motion: reduce) {
    .sending-indicator { animation: none; border-right-color: currentColor; }
  }
  #outbox-list li:is([data-state="unsent"], [data-state="failed"]) .state { color: var(--jp-error); }
  #outbox-list li[data-state="delivered"] .text,
  #outbox-list li[data-state="unsent"] .text { color: var(--jp-text); }
  #outbox-list .text { margin: 0; overflow-wrap: anywhere; white-space: pre-wrap; }

  #outbox-list .again {
    align-self: flex-start;
    padding: calc(3 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 4px;
    background: var(--jp-hover);
    color: var(--jp-text);
    font: calc(11 * var(--jp-unit)) var(--jp-font);
  }


  #panel, #pdf-sections {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(58 * var(--jp-unit));
    z-index: 2147483644;
    width: min(calc(320 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit)));
    max-height: calc(100dvh - 72 * var(--jp-unit));
    overflow: auto;
    padding: calc(6 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(12 * var(--jp-unit))/1.4 var(--jp-font);
  }

  #panel { width: min(calc(360 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit))); }
  #panel[hidden], #pdf-sections[hidden] { display: none; }

  #pdf-sections {
    max-height: min(60vh, calc(100vh - 72 * var(--jp-unit)));
    overflow: auto;
    padding: calc(10 * var(--jp-unit));
  }

  #pdf-sections h2 { margin: 0 calc(6 * var(--jp-unit)) calc(6 * var(--jp-unit)); font: inherit; font-weight: 600; }
  #pdf-sections ul { list-style: none; padding: 0; margin: 0; }
  #pdf-sections li > ul { margin-inline-start: calc(12 * var(--jp-unit)); }
  .pdf-section-row { display: flex; align-items: flex-start; }
  .pdf-section-row button {
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
    padding: calc(5 * var(--jp-unit)) calc(6 * var(--jp-unit));
    cursor: pointer;
  }
  .pdf-section-link { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .pdf-section-row button:disabled { color: var(--jp-muted); cursor: default; }
  .pdf-section-row button:not(:disabled):hover { background: var(--jp-hover); }
  .pdf-section-row button[aria-current="location"] { background: var(--jp-selected); }
  :is(#pdf-outline, #pdf-sections button):focus-visible {
    outline: 2px solid var(--jp-link);
    outline-offset: -2px;
  }

  .control {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--jp-unit));
    padding: calc(9 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border-radius: 6px;
  }

  :is(#text-size-row, #hide-answers-row), :is(#text-size-row, #hide-answers-row) .segments { flex-wrap: wrap; }

  #hide-answers-row .segments { margin-inline-start: auto; }
  #hide-answers-row .segment { padding-inline: calc(5 * var(--jp-unit)); white-space: nowrap; }

  .control .label { flex-grow: 1; cursor: pointer; font-weight: 400; margin: 0; }

  .segments {
    display: flex;
    gap: calc(2 * var(--jp-unit));
    padding: calc(2 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 7px;
    background: var(--jp-field);
  }

  .segment {
    min-height: calc(24 * var(--jp-unit));
    padding: calc(3 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--jp-muted);
    cursor: pointer;
    font: 600 calc(12 * var(--jp-unit))/1 var(--jp-font);
  }

  .corner { width: calc(25 * var(--jp-unit)); padding: 0; font-size: calc(14 * var(--jp-unit)); }
  .segment:hover { background: var(--jp-hover); }
  .segment[aria-pressed="true"] { background: var(--jp-accent); color: var(--jp-on-accent); }

  /* The chosen corner (Panel sets data-corner on the host): what sits at
     the bottom right by default moves to that corner. */
  :host([data-corner$="left"]) :is(#bar, #list, #panel, #pdf-sections, #toast) {
    right: auto;
    left: calc(14 * var(--jp-unit));
  }

  :host([data-corner^="top"]) :is(#bar, #list) { bottom: auto; top: calc(14 * var(--jp-unit)); }
  :host([data-corner^="top"]) :is(#panel, #pdf-sections) { bottom: auto; top: calc(58 * var(--jp-unit)); }
  :host([data-corner^="top"]) #toast { bottom: auto; top: calc(52 * var(--jp-unit)); }
  :host([data-corner$="left"]) #agent-general { right: auto; left: 0; align-items: flex-start; }
  :host([data-corner^="top"]) #agent-general { bottom: auto; top: calc(40 * var(--jp-unit) + var(--sections-offset, 0px)); }

  .chip {
    padding: 1px calc(5 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    cursor: pointer;
    font: calc(11 * var(--jp-unit))/1.3 var(--jp-font);
    white-space: nowrap;
  }

  .chip[data-listening="true"] {
    padding: 1px calc(6 * var(--jp-unit));
    border: 1px dashed var(--jp-focus);
    color: var(--jp-focus);
  }

  .switch {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: flex-start;
    width: calc(28 * var(--jp-unit));
    height: calc(16 * var(--jp-unit));
    padding: calc(2 * var(--jp-unit));
    border: 0;
    border-radius: 999px;
    background: var(--jp-border);
    cursor: pointer;
  }

  .switch::after {
    content: "";
    display: block;
    width: calc(12 * var(--jp-unit));
    height: calc(12 * var(--jp-unit));
    border-radius: 50%;
    background: var(--jp-muted);
  }

  .switch[aria-checked="true"] { justify-content: flex-end; background: var(--jp-switch-on); }
  .switch[aria-checked="true"]::after { background: var(--jp-on-accent); }

  #name-row { display: flex; align-items: center; gap: calc(8 * var(--jp-unit)); padding: calc(6 * var(--jp-unit)) calc(10 * var(--jp-unit)); }
  #name-row label { color: var(--jp-muted); font-weight: 400; margin: 0; }

  #name-field {
    flex-grow: 1;
    min-width: 0;
    padding: calc(4 * var(--jp-unit)) calc(6 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 5px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(12 * var(--jp-unit))/1.3 var(--jp-font);
  }

  #name-field:focus { outline: 1px solid var(--jp-focus); }

  #panel-message {
    padding: 0 calc(10 * var(--jp-unit)) calc(6 * var(--jp-unit));
    color: var(--jp-warning);
    font-size: calc(11 * var(--jp-unit));
  }

  #panel-message:empty { display: none; }

  #people {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(10 * var(--jp-unit));
    margin-top: calc(4 * var(--jp-unit));
    padding: calc(10 * var(--jp-unit)) calc(10 * var(--jp-unit)) calc(6 * var(--jp-unit));
    border-top: 1px solid var(--jp-border);
  }

  #here-count { color: var(--jp-muted); font-size: calc(11 * var(--jp-unit)); }

  #avatars {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .avatar {
    display: flex;
    align-items: center;
    justify-content: center;
    width: calc(24 * var(--jp-unit));
    height: calc(24 * var(--jp-unit));
    border: 2px solid var(--jp-surface);
    border-radius: 50%;
    color: var(--jp-cursor-text);
    font: 700 calc(10 * var(--jp-unit))/1 var(--jp-font);
  }

  .avatar + .avatar { margin-left: calc(-6 * var(--jp-unit)); }
  .avatar[data-you="true"] {
    border-color: var(--jp-text);
    background: var(--jp-surface);
    color: var(--jp-text);
  }

  #cursors {
    position: fixed;
    inset: 0;
    z-index: 2147483642;
    overflow: hidden;
    pointer-events: none;
  }

  .cursor {
    position: absolute;
    left: 0;
    top: 0;
    width: calc(120 * var(--jp-unit));
    height: calc(48 * var(--jp-unit));
    opacity: 1;
    transition: opacity 400ms;
    will-change: transform;
  }

  .cursor[data-faded="true"] { opacity: 0; }
  .cursor svg { position: absolute; left: 0; top: 0; filter: drop-shadow(0 1px calc(2 * var(--jp-unit)) rgb(0 0 0 / 30%)); }

  .cursor .tag {
    position: absolute;
    left: calc(13 * var(--jp-unit));
    top: calc(17 * var(--jp-unit));
    padding: calc(3 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border-radius: 0 calc(6 * var(--jp-unit)) calc(6 * var(--jp-unit)) calc(6 * var(--jp-unit));
    color: var(--jp-cursor-text);
    box-shadow: 0 1px calc(3 * var(--jp-unit)) rgb(0 0 0 / 25%);
    font: 600 calc(11 * var(--jp-unit))/1.3 var(--jp-font);
    white-space: nowrap;
  }

  #name-dialog {
    top: 50%;
    left: 50%;
    width: min(calc(400 * var(--jp-unit)), calc(100vw - 24 * var(--jp-unit)));
    padding: calc(22 * var(--jp-unit));
    border-radius: var(--jp-radius);
    font: calc(13 * var(--jp-unit))/1.45 var(--jp-font);
    transform: translate(-50%, -50%);
  }

  #name-dialog::backdrop { background: rgb(0 0 0 / 35%); }
  #name-dialog form { display: flex; flex-direction: column; gap: calc(14 * var(--jp-unit)); padding: 0; }
  #name-title { display: flex; flex-direction: column; gap: calc(4 * var(--jp-unit)); }
  #name-title strong { font: 700 calc(20 * var(--jp-unit)) var(--jp-font); }
  #name-title span, #name-foot span { color: var(--jp-muted); }
  #name-foot { display: flex; align-items: center; justify-content: space-between; gap: calc(10 * var(--jp-unit)); }
  #name-foot span { font-size: calc(11 * var(--jp-unit)); }
  #name-dialog label { margin: 0; }

  #name-input {
    padding: calc(8 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 4px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(14 * var(--jp-unit))/1.5 var(--jp-font);
  }

  #name-error { color: var(--jp-error); font-size: calc(11 * var(--jp-unit)); }
  #name-error:empty { display: none; }

  #toast {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(52 * var(--jp-unit));
    z-index: 2147483644;
    max-width: min(calc(360 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit)));
    padding: calc(9 * var(--jp-unit)) calc(11 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(13 * var(--jp-unit))/1.35 var(--jp-font);
  }

  #toast[hidden] { display: none; }

  #agent-log { display: contents; }

  #agent-general {
    position: fixed;
    right: 0;
    bottom: calc(40 * var(--jp-unit) + var(--sections-offset, 0px));
    z-index: 2147483643;
    display: flex;
    width: min(calc(388 * var(--jp-unit)), 100vw);
    max-height: calc(100dvh - 54 * var(--jp-unit) - var(--sections-offset, 0px));
    flex-direction: column;
    align-items: flex-end;
    gap: calc(8 * var(--jp-unit));
    padding: calc(12 * var(--jp-unit)) calc(14 * var(--jp-unit));
    pointer-events: none;
  }

  #agent-general.scrollable { overflow: auto; }

  #agent-anchored {
    position: absolute;
    top: 0;
    left: 0;
    width: 0;
    height: 0;
    z-index: 2147483643;
    pointer-events: none;
  }

  .agent-message {
    position: relative;
    width: min(calc(340 * var(--jp-unit)), calc(100vw - 16 * var(--jp-unit)));
    max-height: calc(100vh - 16 * var(--jp-unit));
    padding: calc(7 * var(--jp-unit)) calc(12 * var(--jp-unit)) calc(11 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(13 * var(--jp-unit))/1.45 var(--jp-font);
    overflow-wrap: anywhere;
    overflow-y: auto;
    pointer-events: auto;
  }

  .agent-placement { position: absolute; inset: 0; pointer-events: none; }
  .agent-placement.target-offscreen { visibility: hidden; }
  .agent-placement.viewport-anchored { position: fixed; }
  #agent-anchored .agent-message { position: absolute; z-index: 1; }
  #agent-general .agent-message { max-width: 100%; flex: none; }
  #agent-general .manually-placed { position: fixed; }
  .agent-message[hidden], .agent-anchor-marker[hidden], .agent-anchor-layer g[hidden] { display: none; }
  .agent-anchor-marker { position: absolute; pointer-events: none; border-radius: 2px; }
  .agent-anchor-marker.active { background: var(--jp-tint); }
  .agent-anchor-layer {
    position: absolute;
    inset: 0;
    width: 1px;
    height: 1px;
    overflow: visible;
    color: var(--jp-anchor);
    pointer-events: none;
  }
  .agent-anchor-layer .active { color: var(--jp-anchor-active); }
  .agent-anchor-layer path { fill: none; stroke: currentColor; stroke-linecap: round; }
  .agent-anchor-brace { stroke-width: 1.6px; }
  .agent-anchor-line { stroke-width: 1.4px; opacity: .84; }
  .agent-anchor-layer circle { fill: currentColor; }
  .agent-anchor-layer .active .agent-anchor-line { opacity: 1; }
  .agent-message p { margin: 0 0 calc(8 * var(--jp-unit)); }
  .agent-message p:last-child { margin-bottom: 0; }
  .agent-message ul, .agent-message ol { margin: calc(6 * var(--jp-unit)) 0; padding-left: calc(22 * var(--jp-unit)); }
  .agent-message pre { overflow: auto; }
  .agent-message code { font: inherit; background: var(--jp-hover); }
  .agent-show { display: inline-block; margin-top: calc(6 * var(--jp-unit)); }
  .agent-show[hidden] { display: none; }
  .agent-message a { color: var(--jp-link); text-decoration: underline; }
  .agent-message.has-anchor { cursor: pointer; }
  .agent-message.has-anchor:focus-visible {
    outline: 3px solid var(--jp-focus);
    outline-offset: 2px;
  }

  .agent-header { margin: calc(-7 * var(--jp-unit)) calc(-12 * var(--jp-unit)) calc(8 * var(--jp-unit)); }
  #dialog-header { margin: calc(-14 * var(--jp-unit)) calc(-14 * var(--jp-unit)) calc(12 * var(--jp-unit)); }

  :is(.agent-header, #dialog-header) {
    display: flex;
    align-items: center;
    gap: calc(6 * var(--jp-unit));
    padding: calc(4 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border-bottom: 1px solid var(--jp-border);
    border-radius: calc(var(--jp-radius) - 1px) calc(var(--jp-radius) - 1px) 0 0;
    background: var(--jp-surface-header);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  [data-dragging] { cursor: grabbing; }

  .move-control {
    display: grid;
    place-items: center;
    flex: none;
    width: calc(21 * var(--jp-unit));
    height: calc(21 * var(--jp-unit));
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    cursor: inherit;
  }
  .move-control:is(:hover, :focus-visible) {
    background: var(--jp-hover);
    color: var(--jp-text);
  }
  .move-control:focus-visible { outline: 2px solid var(--jp-focus); }

  .agent-dismiss {
    flex: none;
    margin-inline-start: auto;
    width: calc(21 * var(--jp-unit));
    height: calc(21 * var(--jp-unit));
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    font: calc(18 * var(--jp-unit))/1 var(--jp-font);
  }

  .agent-dismiss:hover, .agent-dismiss:focus-visible { background: var(--jp-hover); }

  .agent-meta {
    color: var(--jp-muted);
    font-size: calc(11 * var(--jp-unit));
    font-weight: 700;
  }

  .agent-meta[hidden] { display: none; }

  .agent-unavailable {
    margin-top: calc(8 * var(--jp-unit));
    color: var(--jp-muted);
    font-size: calc(11 * var(--jp-unit));
  }

  .agent-unavailable[hidden] { display: none; }

  #message-highlight {
    position: fixed;
    display: none;
    z-index: 2147483641;
    border: 1px solid var(--jp-anchor);
    border-radius: 3px;
    background: var(--jp-tint);
    pointer-events: none;
  }

  dialog {
    position: fixed;
    inset: auto;
    width: min(calc(420 * var(--jp-unit)), calc(100vw - 24 * var(--jp-unit)));
    max-width: calc(100vw - 24 * var(--jp-unit));
    margin: 0;
    padding: 0;
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
  }

  dialog::backdrop { background: rgb(0 0 0 / 6%); }
  form { padding: calc(14 * var(--jp-unit)); }

  label {
    display: block;
    margin-bottom: calc(8 * var(--jp-unit));
    font-weight: 700;
    line-height: 1.3;
  }

  #dialog-label { font-size: calc(14 * var(--jp-unit)); margin: 0; flex: 1; }

  #selection-preview {
    margin: 0 0 calc(10 * var(--jp-unit));
    padding: calc(8 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border-left: 3px solid var(--jp-anchor);
    background: var(--jp-hover);
  }

  #selection-preview[hidden] { display: none; }

  #selection-label {
    margin-bottom: calc(4 * var(--jp-unit));
    color: var(--jp-muted);
    font-size: calc(10 * var(--jp-unit));
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  #selection-text {
    max-height: calc(96 * var(--jp-unit));
    margin: 0;
    overflow: auto;
    color: var(--jp-text);
    font: calc(12 * var(--jp-unit))/1.4 var(--jp-font);
    white-space: pre-wrap;
  }

  textarea {
    display: block;
    width: 100%;
    min-height: calc(112 * var(--jp-unit));
    resize: vertical;
    appearance: none;
    box-shadow: none;
    padding: calc(9 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 6px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(14 * var(--jp-unit))/1.4 var(--jp-font);
  }

  textarea:focus { border-color: transparent; outline: 2px solid var(--jp-focus); outline-offset: -1px; }

  #jamparty-dialog { max-height: calc(100dvh - 24 * var(--jp-unit)); overflow: auto; }

  #attachments {
    display: flex;
    flex-wrap: wrap;
    gap: calc(6 * var(--jp-unit));
    padding: 0;
    margin: calc(8 * var(--jp-unit)) 0 0;
    list-style: none;
  }

  #attachments[hidden], #missing-attachments[hidden] { display: none; }

  #attachments li {
    display: flex;
    align-items: center;
    gap: calc(5 * var(--jp-unit));
    min-width: 0;
    max-width: 100%;
    padding: calc(2 * var(--jp-unit)) calc(4 * var(--jp-unit)) calc(2 * var(--jp-unit)) calc(8 * var(--jp-unit));
    background: var(--jp-hover);
    border-radius: 5px;
    font-size: calc(12 * var(--jp-unit));
  }

  #attachments .attachment-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  #attachments .attachment-size { white-space: nowrap; }

  #attachments button, #discard-missing {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: inherit;
    cursor: pointer;
  }

  #attachments button:hover, #discard-missing:hover { border-color: var(--jp-input-border); }

  #missing-attachments {
    margin-top: calc(8 * var(--jp-unit));
    font-size: calc(12 * var(--jp-unit));
    color: var(--jp-error);
  }

  #discard-missing { text-decoration: underline; }
  #attach { margin-right: auto; box-shadow: none; }
  #cancel { background: transparent; border-color: transparent; }
  #cancel:hover { background: var(--jp-hover); }
  #attach svg { display: block; }

  :is(#gear, #pdf-outline, #attach) svg { width: calc(16 * var(--jp-unit)); height: calc(16 * var(--jp-unit)); }
  #edit svg { width: calc(15 * var(--jp-unit)); height: calc(15 * var(--jp-unit)); }
  .move-control svg { width: calc(12 * var(--jp-unit)); height: calc(14 * var(--jp-unit)); }
  .cursor svg { width: calc(16 * var(--jp-unit)); height: calc(18 * var(--jp-unit)); }

  #status {
    margin-top: calc(7 * var(--jp-unit));
    color: var(--jp-error);
    font-size: calc(11 * var(--jp-unit));
    line-height: 1.4;
  }

  #status:empty { display: none; }

  #reload-notice {
    margin-top: calc(7 * var(--jp-unit));
    color: var(--jp-muted);
    font-size: calc(11 * var(--jp-unit));
    line-height: 1.4;
  }

  #reload-notice[hidden] { display: none; }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(8 * var(--jp-unit));
    margin-top: calc(10 * var(--jp-unit));
  }

  button {
    padding: calc(7 * var(--jp-unit)) calc(11 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 6px;
    background: var(--jp-field);
    color: var(--jp-text);
    cursor: pointer;
    font: 600 calc(12 * var(--jp-unit))/1 var(--jp-font);
  }

  button[type="submit"] { border-color: var(--jp-accent); background: var(--jp-accent); color: var(--jp-on-accent); }
  button:disabled { cursor: wait; opacity: 0.55; }

  .agent-timeout { position: absolute; bottom: 0; left: 0; right: 0; height: calc(4 * var(--jp-unit));
    background: var(--jp-muted); transform-origin: left; pointer-events: none;
    animation: answer-countdown linear forwards; }
  @keyframes answer-countdown { from { transform: scaleX(1); } to { transform: scaleX(0); } }
  @media (prefers-reduced-motion: reduce) {
    .agent-timeout { animation-timing-function: steps(var(--answer-timeout-steps), end); }
  }
  :is(button, a, input):focus-visible { outline: 2px solid var(--jp-focus); outline-offset: 1px; }

  .agent-choices { display: flex; flex-wrap: wrap; gap: calc(6 * var(--jp-unit)); margin-top: calc(8 * var(--jp-unit)); }
  .agent-choices:empty { display: none; }
  .agent-choices button { border: 1px solid var(--jp-input-border); border-radius: 5px; padding: calc(5 * var(--jp-unit)) calc(9 * var(--jp-unit));
    background: var(--jp-field); color: var(--jp-muted); cursor: pointer; font: inherit; overflow-wrap: anywhere; }
  .agent-choices button:disabled { opacity: .6; cursor: default; }
`;
