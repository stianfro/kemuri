---
layout: doc
pageClass: home-page
title: Kemuri
description: Kemuri is a single-node latency monitor for Linux.
sidebar: false
aside: false
editLink: false
lastUpdated: false
next: false
---

<section class="home-intro">
  <h1>Kemuri</h1>
  <p class="home-intro__summary">
    Kemuri is a single-node latency monitor for Linux. It runs ICMP, HTTP,
    TCP, TLS, and DNS checks. It stores results in SQLite and shows them in
    smoke-style graphs.
  </p>
  <div class="home-actions">
    <a class="home-action home-action--primary" href="./guide/quick-start">Start with Kemuri</a>
    <a class="home-action" href="https://github.com/stianfro/kemuri">View the source</a>
    <a class="home-action" href="https://github.com/stianfro/kemuri/releases">Download a release</a>
  </div>
</section>

<section class="home-section">
  <h2>What Kemuri does</h2>
  <p class="home-section__intro">
    One Kemuri process schedules checks, writes results, evaluates alerts, and
    provides the web UI and HTTP API.
  </p>
  <div class="fact-grid">
    <article class="fact-card">
      <h3>Network probes</h3>
      <p>Run ICMP, HTTP, TCP, TLS, and DNS checks with typed settings.</p>
    </article>
    <article class="fact-card">
      <h3>Local storage</h3>
      <p>Keep rounds, rollups, revisions, and alert events in one SQLite file.</p>
    </article>
    <article class="fact-card">
      <h3>Alerts</h3>
      <p>Evaluate profile-based rules and send webhook or SMTP notifications.</p>
    </article>
    <article class="fact-card">
      <h3>Operational endpoints</h3>
      <p>Use readiness, liveness, Prometheus metrics, SSE, and the version 1 API.</p>
    </article>
  </div>
</section>

<section class="home-section">
  <h2>Web UI</h2>
  <p class="home-section__intro">
    The web UI is part of the Kemuri binary. It does not need a separate web
    server.
  </p>
  <div class="screen-grid">
    <figure>
      <img src="/screenshots/overview.png" alt="Kemuri overview with four healthy targets" loading="lazy">
      <figcaption>The overview groups targets and shows their current state.</figcaption>
    </figure>
    <figure>
      <img src="/screenshots/check-detail.png" alt="Kemuri check page with a smoke graph and recent rounds" loading="lazy">
      <figcaption>The check page shows latency, loss, health failures, and recent rounds.</figcaption>
    </figure>
  </div>
</section>

<section class="home-section install-block">
  <h2>Install</h2>
  <p class="home-section__intro">
    Release archives are available for Linux, macOS, and Windows. The service
    runtime supports Linux.
  </p>

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/stianfro/kemuri/releases/latest/download/kemuri-installer.sh | sh
```

  <p>
    Read the <a href="./guide/installation">installation guide</a> for
    packages, containers, systemd, and ICMP permissions.
  </p>
</section>

<section class="home-section">
  <h2>Scope</h2>
  <p class="home-section__intro">
    Kemuri is for one process on one trusted Linux host. It does not include
    login, high availability, remote probes, or a distributed scheduler. Put
    it behind a trusted reverse proxy when other users can reach the host.
  </p>
</section>

<section class="home-section">
  <h2>Relationship to SmokePing</h2>
  <p class="home-section__intro">
    Kemuri is an independent implementation inspired by
    <a href="https://oss.oetiker.ch/smokeping/">SmokePing</a>. Kemuri does not
    contain SmokePing source code and is not affiliated with the SmokePing
    project. Read the <a href="./project/provenance">source provenance</a> for
    the source and dependency rules.
  </p>
</section>
