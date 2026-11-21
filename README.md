<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rot256/jamparty/main/assets/brand/jamparty-wordmark-dark.svg">
    <img src="assets/brand/jamparty-wordmark.svg" alt="jamparty" width="320">
  </picture>
</h1>

Claude design, but for everything, everyone and everywhere.

Jamparty works with:

- Practically any web framework/language, because it just proxies the dev server.
- Easily with static files.
- PDFs

And provides an experience similar to Claude design: 
you get a live reloading view of the thing you are working on,
if you see something you want changed, you click it, and tell your agent what to change, it changes it and the page reloads.
It works whether you want to work on something with your friends or just jam with your agent by yourself.

## Get Started

Jamparty is centered around the idea of two parties:

- The "share": which shares one/more resources, e.g. files or a dev server.
- The "join": which joins a party and automatically provided with a live view / comment ability.

Joining is deliberately very easy (see below), while sharing depends on what stack/project is being shared,
fortunately, getting it working is the job of your agent and `jamparty` provides the command `jamparty agent`to quickly get them started.

### Install

```sh
cargo install jamparty
```

### Sharing

Just ask your agent to use `jamparty` to share the project, it will figure it out.

### Join

From anywhere else in the world you/your collaborators run: 

```sh
jamparty join <token>
```

This establishes an end-to-end encrypted (QUIC) connection directly to the sharing instance using [iroh](https://iroh.computer).
No signup, no centralized service, works on the local network and without internet (e.g. when using local models).

