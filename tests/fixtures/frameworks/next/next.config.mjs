// Under a coding agent, Next.js may stop the dev server after a response
// to print an upgrade notice addressed to the agent; every later request
// then fails. Keep it serving, and keep Next.js from writing AGENTS.md
// into the copied site.
export default {
  agentRules: false,
  experimental: { agentUpgrade: false },
};
