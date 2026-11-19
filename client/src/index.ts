import { install } from "./main";

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", () => { install(); }, { once: true });
} else {
  install();
}
