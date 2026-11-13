const path = require("node:path");

module.exports = {
  mode: "development",
  entry: "./src/index.js",
  devServer: {
    host: process.env.HOST,
    // A numeric port: the client dials it on the page's host (not the
    // page's port), so it reaches the dev server only through a joiner
    // serving on the same port.
    port: Number(process.env.PORT),
    static: path.join(__dirname, "public"),
    hot: false,
    liveReload: true,
    client: {
      // The tests bind one address (HOST) to stand for another machine;
      // with no host, as usually, the client is told "0.0.0.0", the page's
      // own host. Say so here too.
      webSocketURL: { hostname: "0.0.0.0" },
    },
  },
};
