import { defineConfig } from "vitepress";

export default defineConfig({
  title: "Kemuri",
  description: "Technical documentation for the Kemuri latency monitor.",
  base: "/kemuri/",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,
  head: [
    ["link", { rel: "icon", type: "image/svg+xml", href: "/kemuri/logo.svg" }],
    ["meta", { name: "theme-color", content: "#1457c8" }],
  ],
  sitemap: {
    hostname: "https://stianfro.github.io/kemuri/",
  },
  themeConfig: {
    logo: "/logo.svg",
    siteTitle: "Kemuri",
    nav: [
      { text: "Documentation", link: "/guide/quick-start" },
      { text: "Configuration", link: "/reference/configuration" },
      { text: "Operations", link: "/operations/deployment" },
      { text: "API", link: "/reference/api" },
      {
        text: "1.0.0",
        items: [
          {
            text: "Release notes",
            link: "https://github.com/stianfro/kemuri/releases/tag/v1.0.0",
          },
          {
            text: "All releases",
            link: "https://github.com/stianfro/kemuri/releases",
          },
        ],
      },
    ],
    sidebar: [
      {
        text: "Start",
        items: [
          { text: "Introduction", link: "/" },
          { text: "Install", link: "/guide/installation" },
          { text: "Quick start", link: "/guide/quick-start" },
          { text: "Use the web UI", link: "/guide/web-ui" },
          { text: "Command line", link: "/guide/command-line" },
        ],
      },
      {
        text: "Concepts",
        items: [
          { text: "Architecture", link: "/concepts/architecture" },
          { text: "Checks and rounds", link: "/concepts/checks-and-rounds" },
          { text: "Status model", link: "/concepts/status-model" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "Configuration", link: "/reference/configuration" },
          { text: "Probe settings", link: "/reference/probes" },
          { text: "Alerts and notifiers", link: "/reference/alerts" },
          { text: "HTTP API", link: "/reference/api" },
        ],
      },
      {
        text: "Operations",
        items: [
          { text: "Deployment", link: "/operations/deployment" },
          { text: "Reload", link: "/operations/reload" },
          { text: "Backups and retention", link: "/operations/backups" },
          { text: "Containers", link: "/operations/containers" },
        ],
      },
      {
        text: "Project",
        items: [
          { text: "Development", link: "/project/development" },
          { text: "Load testing", link: "/project/load-testing" },
          { text: "Source provenance", link: "/project/provenance" },
          { text: "Security", link: "/project/security" },
          { text: "Release process", link: "/project/releases" },
        ],
      },
    ],
    socialLinks: [
      { icon: "github", link: "https://github.com/stianfro/kemuri" },
    ],
    search: {
      provider: "local",
    },
    editLink: {
      pattern:
        "https://github.com/stianfro/kemuri/edit/main/site/:path",
      text: "Edit this page on GitHub",
    },
    footer: {
      message: "Kemuri is available under the MIT License.",
      copyright: "Copyright © 2026 Stian Frøystein",
    },
    outline: {
      level: [2, 3],
      label: "On this page",
    },
  },
});
