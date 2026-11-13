// The comment UI's styles, inside its shadow root.
export const styles = `
  :host {
    all: initial;
    color-scheme: light;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
  }

  *, *::before, *::after { box-sizing: border-box; }

  #highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid #eb5e28;
    background: rgb(235 94 40 / 10%);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 90%);
    pointer-events: none;
  }

  #edit-highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid #2f80ed;
    background: rgb(47 128 237 / 8%);
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
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 999px;
    background: #161616;
    color: #fff;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    cursor: pointer;
    font: 600 12px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .dot {
    display: block;
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #9a958e;
  }

  .dot[data-state="green"] { background: #2f9e44; }
  .dot[data-state="amber"] { background: #e8a33d; }
  .dot[data-state="grey"] { background: #9a958e; }

  #gear, #edit {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 50%;
    background: #161616;
    color: #fff;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    cursor: pointer;
  }

  #gear[aria-expanded="true"] { background: #2a2a2a; }
  #edit[aria-pressed="true"] { border-color: #2f80ed; background: #2f80ed; }
  #edit:disabled { opacity: 0.4; cursor: default; }

  #list {
    position: fixed;
    right: 14px;
    bottom: 14px;
    z-index: 2147483644;
    display: flex;
    flex-direction: column;
    width: min(440px, calc(100vw - 28px));
    max-height: min(60vh, calc(100vh - 28px));
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 12px;
    background: #161616;
    color: #fff;
    box-shadow: 0 8px 28px rgb(0 0 0 / 28%);
    font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
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
    border-bottom: 1px solid #2c2c2c;
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
    color: #b8b2a9;
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  #connection-text[data-state="grey"], #connection-text[data-state="amber"] { color: #f0c27a; }

  #collapse {
    flex: none;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #b8b2a9;
    font: 16px/1 ui-monospace, Menlo, monospace;
  }

  #collapse:hover, #collapse:focus-visible { background: #2a2a2a; }

  #outbox-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 4px 0;
    overflow: auto;
    list-style: none;
  }

  #outbox-list li {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 14px;
    border-bottom: 1px solid #242424;
  }

  #outbox-list li:last-child { border-bottom: 0; }
  #outbox-list .row { display: flex; justify-content: space-between; gap: 10px; }
  #outbox-list .where, #outbox-list .state { color: #b8b2a9; }
  #outbox-list .where { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #outbox-list li[data-state="delivered"] .state { color: #8ce99a; }
  #outbox-list li[data-state="unsent"] .state { color: #ffa8a8; }
  #outbox-list li[data-state="delivered"] .text,
  #outbox-list li[data-state="unsent"] .text { color: #d8d4cd; }
  #outbox-list .text { margin: 0; overflow-wrap: anywhere; white-space: pre-wrap; }

  #outbox-list .again {
    align-self: flex-start;
    padding: 3px 8px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #2a2a2a;
    color: #fff;
    font: 11px ui-monospace, Menlo, monospace;
  }


  #panel {
    position: fixed;
    right: 14px;
    bottom: 58px;
    z-index: 2147483644;
    width: 232px;
    padding: 6px;
    border: 1px solid rgb(255 255 255 / 12%);
    border-radius: 10px;
    background: #161616;
    color: #fff;
    box-shadow: 0 6px 20px rgb(0 0 0 / 22%);
    font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #panel[hidden] { display: none; }

  .control {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 10px;
    border-radius: 6px;
  }

  .control .label { flex-grow: 1; cursor: pointer; }

  .chip {
    padding: 1px 5px;
    border: 1px solid #3a3a3a;
    border-radius: 4px;
    background: transparent;
    color: #b8b2a9;
    cursor: pointer;
    font: 11px/1.3 ui-monospace, Menlo, monospace;
    white-space: nowrap;
  }

  .chip[data-listening="true"] {
    padding: 1px 6px;
    border: 1px dashed #66d9e8;
    color: #66d9e8;
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
    background: #3a3a3a;
    cursor: pointer;
  }

  .switch::after {
    content: "";
    display: block;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #b8b2a9;
  }

  .switch[aria-checked="true"] { justify-content: flex-end; background: #2f9e44; }
  .switch[aria-checked="true"]::after { background: #fff; }

  #name-row { display: flex; align-items: center; gap: 8px; padding: 6px 10px; }
  #name-row label { color: #b8b2a9; font-weight: 400; margin: 0; }

  #name-field {
    flex-grow: 1;
    min-width: 0;
    padding: 4px 6px;
    border: 1px solid #3a3a3a;
    border-radius: 4px;
    background: #222;
    color: #fff;
    font: 12px/1.3 ui-monospace, Menlo, monospace;
  }

  #name-field:focus { outline: 1px solid #66d9e8; }

  #panel-message {
    padding: 0 10px 6px;
    color: #f0c27a;
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
    border-top: 1px solid #262626;
  }

  #here-count { color: #8c867e; font-size: 11px; }

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
    border: 2px solid #161616;
    border-radius: 50%;
    color: #fff;
    font: 700 10px/1 system-ui, -apple-system, sans-serif;
  }

  .avatar + .avatar { margin-left: -6px; }
  .avatar[data-you="true"] { border-color: #fff; background: #161616; }

  #notice {
    position: fixed;
    right: 14px;
    bottom: 52px;
    z-index: 2147483644;
    max-width: min(420px, calc(100vw - 28px));
    padding: 9px 11px;
    border-radius: 6px;
    background: #161616;
    color: #f0c27a;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #notice[hidden] { display: none; }

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
    color: #fff;
    box-shadow: 0 1px 3px rgb(0 0 0 / 25%);
    font: 600 11px/1.3 system-ui, -apple-system, sans-serif;
    white-space: nowrap;
  }

  #name-dialog {
    top: 50%;
    left: 50%;
    width: min(400px, calc(100vw - 24px));
    padding: 22px;
    border-radius: 10px;
    font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    transform: translate(-50%, -50%);
  }

  #name-dialog::backdrop { background: rgb(0 0 0 / 35%); }
  #name-dialog form { display: flex; flex-direction: column; gap: 14px; padding: 0; }
  #name-title { display: flex; flex-direction: column; gap: 4px; }
  #name-title strong { font: 700 20px Georgia, serif; }
  #name-title span, #name-foot span { color: #5b564f; }
  #name-foot { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  #name-foot span { font-size: 11px; }
  #name-dialog label { margin: 0; }

  #name-input {
    padding: 8px 10px;
    border: 2px solid #287088;
    border-radius: 4px;
    background: #fff;
    color: #161616;
    font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #name-error { color: #a12c10; font-size: 11px; }
  #name-error:empty { display: none; }

  #toast {
    position: fixed;
    right: 14px;
    bottom: 52px;
    z-index: 2147483644;
    max-width: min(360px, calc(100vw - 28px));
    padding: 9px 11px;
    border-radius: 5px;
    background: #161616;
    color: #fff;
    box-shadow: 0 4px 18px rgb(0 0 0 / 24%);
    font: 13px/1.35 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #toast[hidden] { display: none; }

  #agent-log { display: contents; }

  #agent-general {
    position: fixed;
    right: 14px;
    bottom: 52px;
    z-index: 2147483643;
    display: flex;
    width: min(360px, calc(100vw - 28px));
    max-height: calc(100vh - 66px);
    flex-direction: column;
    gap: 8px;
    overflow: auto;
    pointer-events: none;
  }

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
    padding: 12px 34px 12px 13px;
    border: 1px solid #45413c;
    border-radius: 7px;
    background: #fffdf8;
    color: #1d1b19;
    box-shadow: 0 6px 24px rgb(0 0 0 / 24%);
    font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    overflow-wrap: anywhere;
    overflow-y: auto;
    pointer-events: auto;
  }

  #agent-anchored .agent-message { position: fixed; }
  .agent-message p { margin: 0 0 8px; }
  .agent-message p:last-child { margin-bottom: 0; }
  .agent-message ul, .agent-message ol { margin: 6px 0; padding-left: 22px; }
  .agent-message pre { overflow: auto; }
  .agent-message code { font: inherit; background: #eeeae2; }
  .agent-message a { color: #075f7a; text-decoration: underline; }
  .agent-message.has-anchor { cursor: pointer; }
  .agent-message.has-anchor:focus-visible {
    outline: 3px solid #287088;
    outline-offset: 2px;
  }

  .agent-dismiss {
    position: absolute;
    top: 5px;
    right: 5px;
    width: 25px;
    height: 25px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: #504b45;
    font: 18px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .agent-dismiss:hover, .agent-dismiss:focus-visible { background: #eeeae2; }

  .agent-meta {
    margin-bottom: 4px;
    color: #7b3221;
    font-size: 11px;
    font-weight: 700;
  }

  .agent-meta[hidden] { display: none; }

  .agent-unavailable {
    margin-top: 8px;
    color: #7b3221;
    font-size: 11px;
  }

  .agent-unavailable[hidden] { display: none; }

  #message-highlight {
    position: fixed;
    display: none;
    z-index: 2147483646;
    border: 3px solid #0b7896;
    border-radius: 3px;
    background: rgb(11 120 150 / 12%);
    box-shadow: 0 0 0 3px rgb(255 255 255 / 85%);
    pointer-events: none;
  }

  dialog {
    position: fixed;
    inset: auto;
    width: min(420px, calc(100vw - 24px));
    margin: 0;
    padding: 0;
    border: 1px solid #444;
    border-radius: 7px;
    background: #f8f7f4;
    color: #161616;
    box-shadow: 0 16px 48px rgb(0 0 0 / 28%);
  }

  dialog::backdrop { background: rgb(0 0 0 / 6%); }
  form { padding: 14px; }

  label {
    display: block;
    margin-bottom: 8px;
    font-weight: 700;
    line-height: 1.3;
  }

  #selection-preview {
    margin: 0 0 10px;
    padding: 8px 10px;
    border-left: 3px solid #eb5e28;
    background: #eeeae2;
  }

  #selection-preview[hidden] { display: none; }

  #selection-label {
    margin-bottom: 4px;
    color: #655f58;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  #selection-text {
    max-height: 96px;
    margin: 0;
    overflow: auto;
    color: #2b2926;
    font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    white-space: pre-wrap;
  }

  textarea {
    display: block;
    width: 100%;
    min-height: 112px;
    resize: vertical;
    padding: 9px 10px;
    border: 1px solid #888;
    border-radius: 4px;
    background: #fff;
    color: #161616;
    font: 14px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  textarea:focus { outline: 2px solid #287088; outline-offset: 1px; }

  #status {
    min-height: 18px;
    margin-top: 7px;
    color: #a12c10;
    font-size: 11px;
    line-height: 1.4;
  }

  #reload-notice {
    margin-top: 7px;
    color: #655f58;
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
    border: 1px solid #555;
    border-radius: 4px;
    background: #fff;
    color: #161616;
    cursor: pointer;
    font: 600 12px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  button[type="submit"] { background: #161616; color: #fff; }
  button:disabled { cursor: wait; opacity: 0.55; }
`;
