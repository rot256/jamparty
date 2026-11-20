// The comment UI's styles, inside its shadow root.
const dark = `
    color-scheme: dark;
    --jp-surface: #161616;
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
    --jp-shadow: 0 8px 24px rgb(0 0 0 / 26%);
  `;

export const styles = `
  :host { all: initial; }

  #overlay-root {
    all: initial;
    display: contents;
    color-scheme: light;
    --jp-font: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    font-family: var(--jp-font);
    --jp-surface: #fff;
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
    --jp-shadow: 0 8px 24px rgb(28 40 60 / 16%);
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
    --jp-radius: 11px;
    font-size: 13px;
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
    right: 14px;
    bottom: 14px;
    z-index: 2147483644;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  #bar[hidden] { display: none; }

  #pill {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px 7px 9px;
    border: 1px solid var(--jp-border);
    border-radius: 999px;
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    cursor: pointer;
    font: 600 12px/1 var(--jp-font);
  }

  .dot {
    display: block;
    flex: none;
    width: 10px;
    height: 10px;
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
    width: 32px;
    height: 32px;
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
    right: 14px;
    bottom: 14px;
    z-index: 2147483644;
    display: flex;
    flex-direction: column;
    width: min(440px, calc(100vw - 28px));
    max-height: min(60vh, calc(100vh - 28px));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: 12px/1.45 var(--jp-font);
    overflow: hidden;
  }

  #list[hidden] { display: none; }

  #list header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--jp-border);
    cursor: pointer;
  }

  #list-summary {
    display: flex;
    align-items: center;
    gap: 8px;
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
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--jp-muted);
    font: 16px/1 var(--jp-font);
  }

  #collapse:hover, #collapse:focus-visible { background: var(--jp-hover); }

  #list-content { overflow: auto; }

  #outbox-list, #answer-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 4px 0;
    list-style: none;
  }

  #outbox-list > li, #answer-list > li {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--jp-border);
  }

  #outbox-list > li:last-child, #answer-list > li:last-child { border-bottom: 0; }
  #outbox-list:empty, #answer-list:empty { display: none; }
  #outbox-list:not(:empty) + #answer-list:not(:empty) { border-top: 1px solid var(--jp-border); }

  .agent-reply { min-width: 0; color: var(--jp-text); overflow-wrap: anywhere; }
  #outbox-list .agent-reply {
    margin-top: 4px;
    padding-inline-start: 8px;
    border-inline-start: 2px solid var(--jp-border);
  }
  .agent-reply p { margin: 0 0 4px; }
  .agent-reply p:last-child { margin-bottom: 0; }
  .agent-reply a { color: var(--jp-link); }
  .agent-reply pre { overflow: auto; }
  .agent-reply table { display: block; overflow: auto; }
  #outbox-list .row { display: flex; justify-content: space-between; gap: 10px; }
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
    padding: 3px 8px;
    border: 1px solid var(--jp-border);
    border-radius: 4px;
    background: var(--jp-hover);
    color: var(--jp-text);
    font: 11px var(--jp-font);
  }


  #panel, #pdf-sections {
    position: fixed;
    right: 14px;
    bottom: 58px;
    z-index: 2147483644;
    width: min(260px, calc(100vw - 28px));
    max-height: calc(100dvh - 72px);
    overflow: auto;
    padding: 6px;
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: 12px/1.4 var(--jp-font);
  }

  #panel[hidden], #pdf-sections[hidden] { display: none; }

  #pdf-sections {
    width: min(320px, calc(100vw - 28px));
    max-height: min(60vh, calc(100vh - 72px));
    overflow: auto;
    padding: 10px;
  }

  #pdf-sections h2 { margin: 0 6px 6px; font: inherit; font-weight: 600; }
  #pdf-sections ul { list-style: none; padding: 0; margin: 0; }
  #pdf-sections li > ul { margin-inline-start: 12px; }
  .pdf-section-row { display: flex; align-items: flex-start; }
  .pdf-section-row button {
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
    padding: 5px 6px;
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
    gap: 8px;
    padding: 9px 10px;
    border-radius: 6px;
  }

  .control .label { flex-grow: 1; cursor: pointer; }

  .segments {
    display: flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--jp-border);
    border-radius: 7px;
    background: var(--jp-field);
  }

  .segment {
    min-height: 24px;
    padding: 3px 8px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--jp-muted);
    cursor: pointer;
    font: 600 12px/1 var(--jp-font);
  }

  .corner { width: 25px; padding: 0; font-size: 14px; }
  .segment:hover { background: var(--jp-hover); }
  .segment[aria-pressed="true"] { background: var(--jp-accent); color: var(--jp-on-accent); }

  /* The chosen corner (Panel sets data-corner on the host): what sits at
     the bottom right by default moves to that corner. */
  :host([data-corner$="left"]) :is(#bar, #list, #panel, #pdf-sections, #toast) {
    right: auto;
    left: 14px;
  }

  :host([data-corner^="top"]) :is(#bar, #list) { bottom: auto; top: 14px; }
  :host([data-corner^="top"]) :is(#panel, #pdf-sections) { bottom: auto; top: 58px; }
  :host([data-corner^="top"]) #toast { bottom: auto; top: 52px; }
  :host([data-corner$="left"]) #agent-general { right: auto; left: 0; align-items: flex-start; }
  :host([data-corner^="top"]) #agent-general { bottom: auto; top: calc(40px + var(--sections-offset, 0px)); }

  .chip {
    padding: 1px 5px;
    border: 1px solid var(--jp-border);
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    cursor: pointer;
    font: 11px/1.3 var(--jp-font);
    white-space: nowrap;
  }

  .chip[data-listening="true"] {
    padding: 1px 6px;
    border: 1px dashed var(--jp-focus);
    color: var(--jp-focus);
  }

  .switch {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: flex-start;
    width: 28px;
    height: 16px;
    padding: 2px;
    border: 0;
    border-radius: 999px;
    background: var(--jp-border);
    cursor: pointer;
  }

  .switch::after {
    content: "";
    display: block;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--jp-muted);
  }

  .switch[aria-checked="true"] { justify-content: flex-end; background: var(--jp-switch-on); }
  .switch[aria-checked="true"]::after { background: var(--jp-on-accent); }

  #name-row { display: flex; align-items: center; gap: 8px; padding: 6px 10px; }
  #name-row label { color: var(--jp-muted); font-weight: 400; margin: 0; }

  #name-field {
    flex-grow: 1;
    min-width: 0;
    padding: 4px 6px;
    border: 1px solid var(--jp-input-border);
    border-radius: 5px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: 12px/1.3 var(--jp-font);
  }

  #name-field:focus { outline: 1px solid var(--jp-focus); }

  #panel-message {
    padding: 0 10px 6px;
    color: var(--jp-warning);
    font-size: 11px;
  }

  #panel-message:empty { display: none; }

  #people {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 4px;
    padding: 10px 10px 6px;
    border-top: 1px solid var(--jp-border);
  }

  #here-count { color: var(--jp-muted); font-size: 11px; }

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
    width: 24px;
    height: 24px;
    border: 2px solid var(--jp-surface);
    border-radius: 50%;
    color: var(--jp-cursor-text);
    font: 700 10px/1 var(--jp-font);
  }

  .avatar + .avatar { margin-left: -6px; }
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
    width: 120px;
    height: 48px;
    opacity: 1;
    transition: opacity 400ms;
    will-change: transform;
  }

  .cursor[data-faded="true"] { opacity: 0; }
  .cursor svg { position: absolute; left: 0; top: 0; filter: drop-shadow(0 1px 2px rgb(0 0 0 / 30%)); }

  .cursor .tag {
    position: absolute;
    left: 13px;
    top: 17px;
    padding: 3px 8px;
    border-radius: 0 6px 6px 6px;
    color: var(--jp-cursor-text);
    box-shadow: 0 1px 3px rgb(0 0 0 / 25%);
    font: 600 11px/1.3 var(--jp-font);
    white-space: nowrap;
  }

  #name-dialog {
    top: 50%;
    left: 50%;
    width: min(400px, calc(100vw - 24px));
    padding: 22px;
    border-radius: var(--jp-radius);
    font: 13px/1.45 var(--jp-font);
    transform: translate(-50%, -50%);
  }

  #name-dialog::backdrop { background: rgb(0 0 0 / 35%); }
  #name-dialog form { display: flex; flex-direction: column; gap: 14px; padding: 0; }
  #name-title { display: flex; flex-direction: column; gap: 4px; }
  #name-title strong { font: 700 20px var(--jp-font); }
  #name-title span, #name-foot span { color: var(--jp-muted); }
  #name-foot { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  #name-foot span { font-size: 11px; }
  #name-dialog label { margin: 0; }

  #name-input {
    padding: 8px 10px;
    border: 1px solid var(--jp-input-border);
    border-radius: 4px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: 14px/1.5 var(--jp-font);
  }

  #name-error { color: var(--jp-error); font-size: 11px; }
  #name-error:empty { display: none; }

  #toast {
    position: fixed;
    right: 14px;
    bottom: 52px;
    z-index: 2147483644;
    max-width: min(360px, calc(100vw - 28px));
    padding: 9px 11px;
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: 13px/1.35 var(--jp-font);
  }

  #toast[hidden] { display: none; }

  #agent-log { display: contents; }

  #agent-general {
    position: fixed;
    right: 0;
    bottom: calc(40px + var(--sections-offset, 0px));
    z-index: 2147483643;
    display: flex;
    width: min(388px, 100vw);
    max-height: calc(100dvh - 54px - var(--sections-offset, 0px));
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    padding: 12px 14px;
    pointer-events: none;
  }

  #agent-general.scrollable { overflow: auto; }

  #agent-anchored {
    position: fixed;
    inset: 0;
    z-index: 2147483643;
    pointer-events: none;
  }

  .agent-message {
    position: relative;
    width: min(340px, calc(100vw - 16px));
    max-height: calc(100vh - 16px);
    padding: 7px 12px 11px;
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: 13px/1.45 var(--jp-font);
    overflow-wrap: anywhere;
    overflow-y: auto;
    pointer-events: auto;
  }

  #agent-anchored .agent-message { position: fixed; z-index: 1; }
  #agent-general .agent-message { max-width: 100%; flex: none; }
  #agent-general .manually-placed { position: fixed; }
  .agent-message[hidden], .agent-anchor-marker[hidden], .agent-anchor-layer g[hidden] { display: none; }
  .agent-anchor-marker { position: fixed; pointer-events: none; border-radius: 2px; }
  .agent-anchor-marker.active { background: var(--jp-tint); }
  .agent-anchor-layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    color: var(--jp-anchor);
    pointer-events: none;
  }
  .agent-anchor-layer .active { color: var(--jp-anchor-active); }
  .agent-anchor-layer path { fill: none; stroke: currentColor; stroke-linecap: round; }
  .agent-anchor-brace { stroke-width: 1.6px; }
  .agent-anchor-line { stroke-width: 1.4px; opacity: .84; }
  .agent-anchor-layer circle { fill: currentColor; }
  .agent-anchor-layer .active .agent-anchor-line { opacity: 1; }
  .agent-message p { margin: 0 0 8px; }
  .agent-message p:last-child { margin-bottom: 0; }
  .agent-message ul, .agent-message ol { margin: 6px 0; padding-left: 22px; }
  .agent-message pre { overflow: auto; }
  .agent-message code { font: inherit; background: var(--jp-hover); }
  .agent-show { display: inline-block; margin-top: 6px; }
  .agent-show[hidden] { display: none; }
  .agent-message a { color: var(--jp-link); text-decoration: underline; }
  .agent-message.has-anchor { cursor: pointer; }
  .agent-message.has-anchor:focus-visible {
    outline: 3px solid var(--jp-focus);
    outline-offset: 2px;
  }

  .agent-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 21px;
    gap: 6px;
    margin: -2px -6px 2px 0;
  }

  :is(.agent-header, #dialog-header) {
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  [data-dragging] { cursor: grabbing; }

  .move-control {
    display: grid;
    place-items: center;
    flex: none;
    width: 21px;
    height: 21px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
  }

  .move-control:hover { background: var(--jp-hover); }
  .move-control:focus-visible { outline: 2px solid var(--jp-focus); }

  .agent-dismiss {
    flex: none;
    margin-inline-start: auto;
    width: 21px;
    height: 21px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    font: 18px/1 var(--jp-font);
  }

  .agent-dismiss:hover, .agent-dismiss:focus-visible { background: var(--jp-hover); }

  .agent-meta {
    color: var(--jp-muted);
    font-size: 11px;
    font-weight: 700;
  }

  .agent-meta[hidden] { display: none; }

  .agent-unavailable {
    margin-top: 8px;
    color: var(--jp-muted);
    font-size: 11px;
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
    width: min(420px, calc(100vw - 24px));
    max-width: calc(100vw - 24px);
    margin: 0;
    padding: 0;
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
  }

  dialog::backdrop { background: rgb(0 0 0 / 6%); }
  form { padding: 14px; }

  label {
    display: block;
    margin-bottom: 8px;
    font-weight: 700;
    line-height: 1.3;
  }

  #dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    margin-bottom: 8px;
  }

  #dialog-label { font-size: 14px; margin: 0; flex: 1; }

  #selection-preview {
    margin: 0 0 10px;
    padding: 8px 10px;
    border-left: 3px solid var(--jp-anchor);
    background: var(--jp-hover);
  }

  #selection-preview[hidden] { display: none; }

  #selection-label {
    margin-bottom: 4px;
    color: var(--jp-muted);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  #selection-text {
    max-height: 96px;
    margin: 0;
    overflow: auto;
    color: var(--jp-text);
    font: 12px/1.4 var(--jp-font);
    white-space: pre-wrap;
  }

  textarea {
    display: block;
    width: 100%;
    min-height: 112px;
    resize: vertical;
    appearance: none;
    box-shadow: none;
    padding: 9px 10px;
    border: 1px solid var(--jp-input-border);
    border-radius: 6px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: 14px/1.4 var(--jp-font);
  }

  textarea:focus { border-color: transparent; outline: 2px solid var(--jp-focus); outline-offset: -1px; }

  #jamparty-dialog { max-height: calc(100dvh - 24px); overflow: auto; }

  #attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0;
    margin: 8px 0 0;
    list-style: none;
  }

  #attachments[hidden], #missing-attachments[hidden] { display: none; }

  #attachments li {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    max-width: 100%;
    padding: 2px 4px 2px 8px;
    background: var(--jp-hover);
    border-radius: 5px;
    font-size: 12px;
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
    margin-top: 8px;
    font-size: 12px;
    color: var(--jp-error);
  }

  #discard-missing { text-decoration: underline; }
  #attach { margin-right: auto; box-shadow: none; }
  #cancel { background: transparent; border-color: transparent; }
  #cancel:hover { background: var(--jp-hover); }
  #attach svg { display: block; }

  #status {
    margin-top: 7px;
    color: var(--jp-error);
    font-size: 11px;
    line-height: 1.4;
  }

  #status:empty { display: none; }

  #reload-notice {
    margin-top: 7px;
    color: var(--jp-muted);
    font-size: 11px;
    line-height: 1.4;
  }

  #reload-notice[hidden] { display: none; }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }

  button {
    padding: 7px 11px;
    border: 1px solid var(--jp-border);
    border-radius: 6px;
    background: var(--jp-field);
    color: var(--jp-text);
    cursor: pointer;
    font: 600 12px/1 var(--jp-font);
  }

  button[type="submit"] { border-color: var(--jp-accent); background: var(--jp-accent); color: var(--jp-on-accent); }
  button:disabled { cursor: wait; opacity: 0.55; }

  .agent-timeout { position: absolute; bottom: 0; left: 0; right: 0; height: 4px;
    background: var(--jp-muted); transform-origin: left; pointer-events: none;
    animation: answer-countdown linear forwards; }
  @keyframes answer-countdown { from { transform: scaleX(1); } to { transform: scaleX(0); } }
  @media (prefers-reduced-motion: reduce) {
    .agent-timeout { animation-timing-function: steps(var(--answer-timeout-steps), end); }
  }
  #answer-timeout {
    width: 60px;
    min-width: 0;
    padding: 4px 6px;
    border: 1px solid var(--jp-input-border);
    border-radius: 5px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: 12px/1.3 var(--jp-font);
  }

  :is(button, a, input):focus-visible { outline: 2px solid var(--jp-focus); outline-offset: 1px; }

  .agent-choices { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
  .agent-choices:empty { display: none; }
  .agent-choices button { border: 1px solid var(--jp-input-border); border-radius: 5px; padding: 5px 9px;
    background: var(--jp-field); color: var(--jp-muted); cursor: pointer; font: inherit; overflow-wrap: anywhere; }
  .agent-choices button:disabled { opacity: .6; cursor: default; }
`;
