// The connection state the page shows: its socket to the joiner, the
// joiner's link to the share and the share's status line.

import type { Link } from "./generated/api";

export type Colour = "green" | "amber" | "grey";

export interface ConnectionView {
  colour: Colour;
  /** What the state means, for people. */
  text: string;
}

export class Connection {
  private socketOpen = false;
  private link: Link | null = null;
  /** The share is reachable through the joiner, status line or not. */
  private reachable = false;

  constructor(
    /**
     * The state changed; `reached` when the share became reachable again
     * (comments can go out, even with a status line showing).
     */
    private readonly changed: (view: ConnectionView, reached: boolean) => void,
  ) {}

  get view(): ConnectionView {
    let colour: Colour = "green";
    let text = "connected";
    if (!this.socketOpen) {
      colour = "amber";
      text = "reconnecting to jamparty join…";
    } else if (this.link?.connected !== true) {
      colour = "grey";
      text = "share unreachable";
    } else if (this.link.status !== null) {
      colour = "grey";
      text = this.link.status;
    }
    if (colour === "green" && this.link?.path) text += ` · ${this.link.path}`;
    return { colour, text };
  }

  setOpen(open: boolean): void {
    this.socketOpen = open;
    this.render();
  }

  setLink(link: Link): void {
    this.link = link;
    this.render();
  }

  private render(): void {
    const reachable = this.socketOpen && this.link?.connected === true;
    const reached = reachable && !this.reachable;
    this.reachable = reachable;
    this.changed(this.view, reached);
  }
}
