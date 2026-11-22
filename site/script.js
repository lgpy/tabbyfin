(() => {
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;

  // Each part of the page stands alone: if one fails, the others still run.
  const part = run => { try { run(); } catch (e) { console.error(e); } };

  part(() => {
    /* ---------- Downloads: the latest release's files, as soon as GitHub answers ---------- */
    const RELEASES = 'https://github.com/lgpy/tabbyfin/releases/latest';
    const PATTERNS = {
      'win-setup': n => n.endsWith('-windows-x86_64-setup.exe'),
      'win-zip': n => n.endsWith('-windows-x86_64.zip'),
      'deb': n => n.endsWith('_amd64.deb'),
      'appimage': n => n.endsWith('-linux-x86_64.AppImage'),
      'flatpak': n => n.endsWith('.flatpak'),
      'tar': n => n.endsWith('-linux-x86_64.tar.gz'),
      'mac-arm': n => n.endsWith('-macos-aarch64.zip'),
      'mac-intel': n => n.endsWith('-macos-x86_64.zip'),
    };
    // Which package or processor the visitor has can't be told, so only the system is guessed.
    const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    const agent = navigator.userAgent || '';
    let os = null;
    if (/android|iphone|ipad|ipod/i.test(agent) || (/mac/i.test(platform) && navigator.maxTouchPoints > 1)) os = null;
    else if (/win/i.test(platform)) os = 'windows';
    else if (/mac/i.test(platform)) os = 'macos';
    else if (/linux|x11|cros/i.test(platform) || /linux/i.test(agent)) os = 'linux';

    const NOTES = {
      windows: 'The installer for Windows 10 and 11.',
      linux: 'Debian and Ubuntu, other distributions, Flatpak or a tarball.',
      macos: 'For Apple silicon or Intel; playback needs mpv from Homebrew.',
    };
    const mainBtn = document.getElementById('download-main');
    const mainNote = document.getElementById('main-note');
    function setMain(assets) {
      if (!os) return;
      document.getElementById('main-label').textContent = 'Download for ' + { windows: 'Windows', linux: 'Linux', macos: 'macOS' }[os];
      document.getElementById('download-' + os).classList.add('yours');
      const a = os === 'windows' && assets && assets['win-setup'];
      mainBtn.href = os === 'windows' ? (a ? a.url : RELEASES) : '#download-' + os;
      mainNote.textContent = NOTES[os] + (a && a.size ? ' ' + a.size + '.' : '');
    }
    const size = bytes => {
      const kb = Math.round(bytes / 1024);
      return kb >= 1024 ? Math.round(bytes / 1048576) + ' MB' : kb >= 1 ? kb + ' KB' : '';
    };
    function apply(rel) {
      const assets = {};
      for (const [key, test] of Object.entries(PATTERNS)) {
        const a = rel.assets.find(a => test(a.name));
        if (a) assets[key] = { url: a.browser_download_url, size: size(a.size), name: a.name };
      }
      document.querySelectorAll('[data-asset]').forEach(link => {
        const a = assets[link.dataset.asset];
        if (!a) return;
        link.href = a.url;
        link.querySelector('.file').textContent = a.name;
        if (a.size) link.querySelector('[data-size]').textContent = a.size;
      });
      for (const key of ['deb', 'flatpak']) {
        document.querySelectorAll(`[data-name="${key}"]`).forEach(s => { if (assets[key]) s.textContent = assets[key].name; });
      }
      const version = (rel.tag_name || '').replace(/^v/, '');
      if (version) document.querySelectorAll('[data-version]').forEach(s => { s.textContent = version; });
      const date = (rel.published_at || '').slice(0, 10);
      if (date) document.querySelectorAll('[data-date]').forEach(s => { s.textContent = ` (${date})`; });
      setMain(assets);
    }
    setMain(null);
    // The answer is trusted only this far: a list of files, each a download of this repository's
    // releases. Anything else (a rate limit's message, a cache someone wrote to) leaves the
    // links on the releases page.
    // The address is checked as the browser will read it, so a path with ".." in it can't climb out.
    const ours = url => {
      try {
        const u = new URL(url);
        return u.origin === 'https://github.com' && u.pathname.startsWith('/lgpy/tabbyfin/releases/download/') && u.href === url;
      } catch (e) { return false; }
    };
    const text = v => v == null || typeof v === 'string';
    const valid = rel => !!rel && text(rel.tag_name) && text(rel.published_at) && Array.isArray(rel.assets) && rel.assets.every(a => a && typeof a.name === 'string'
      && typeof a.browser_download_url === 'string' && ours(a.browser_download_url)
      && typeof a.size === 'number' && a.size >= 0);
    let cached = null;
    try { cached = JSON.parse(sessionStorage.getItem('tabbyfin-release')); } catch (e) {}
    if (valid(cached)) apply(cached);
    else {
      fetch('https://api.github.com/repos/lgpy/tabbyfin/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
        .then(r => r.ok ? r.json() : Promise.reject(r.status))
        .then(rel => {
          if (!valid(rel)) return;
          const slim = {
            tag_name: rel.tag_name, published_at: rel.published_at,
            assets: rel.assets.map(a => ({ name: a.name, size: a.size, browser_download_url: a.browser_download_url })),
          };
          try { sessionStorage.setItem('tabbyfin-release', JSON.stringify(slim)); } catch (e) {}
          apply(slim);
        })
        .catch(() => {});
    }
  });

  part(() => {
    /* ---------- Theme: system, light or dark, kept in localStorage (theme.js applies it before first paint) ---------- */
    const picks = [...document.querySelectorAll('[data-theme-pick]')];
    const setTheme = value => {
      if (value === 'system') delete document.documentElement.dataset.theme;
      else document.documentElement.dataset.theme = value;
      picks.forEach(b => b.setAttribute('aria-pressed', String(b.dataset.themePick === value)));
      try { value === 'system' ? localStorage.removeItem('tabbyfin-theme') : localStorage.setItem('tabbyfin-theme', value); } catch (e) {}
    };
    picks.forEach(b => b.addEventListener('click', () => setTheme(b.dataset.themePick)));
    setTheme(document.documentElement.dataset.theme || 'system');
    // A choice made in another tab of the site reaches this one.
    addEventListener('storage', e => {
      if (e.key !== 'tabbyfin-theme' && e.key !== null) return;
      setTheme(e.newValue === 'light' || e.newValue === 'dark' ? e.newValue : 'system');
    });
  });

  part(() => {
    /* ---------- Screens: the strip becomes tabs over the window ---------- */
    const strip = document.getElementById('strip');
    const tabs = [...strip.querySelectorAll('a')];
    const panes = Object.fromEntries(tabs.map(t => [t.dataset.screen, document.getElementById('screen-' + t.dataset.screen)]));
    const stage = document.getElementById('stage');
    const winTitle = document.getElementById('win-title');
    const caption = document.getElementById('stage-caption');
    strip.setAttribute('role', 'tablist');
    strip.setAttribute('aria-label', 'Screens of the app');
    strip.querySelectorAll('li').forEach(li => li.setAttribute('role', 'presentation'));
    tabs.forEach(t => {
      const pane = panes[t.dataset.screen];
      t.id = 'tab-' + t.dataset.screen;
      t.setAttribute('role', 'tab');
      t.setAttribute('aria-controls', pane.id);
      pane.setAttribute('role', 'tabpanel');
      pane.setAttribute('aria-labelledby', t.id);
    });
    let current = 'home';
    function show(name, focus) {
      current = name;
      tabs.forEach(t => {
        const on = t.dataset.screen === name;
        t.setAttribute('aria-selected', on);
        t.tabIndex = on ? 0 : -1;
        if (on && focus) t.focus();
      });
      jelly.react();
      const pane = panes[name], img = pane.querySelector('img');
      const swap = () => {
        if (current !== name) return; // another screen was chosen while this one loaded
        tabs.forEach(t => { panes[t.dataset.screen].hidden = t.dataset.screen !== name; });
        winTitle.textContent = pane.dataset.name;
        caption.textContent = img.alt.replace(/^Tabbyfin's /, '');
        // The player is dark over the video, so the lights go down for it.
        stage.classList.toggle('lights-down', name === 'player');
      };
      // The screens that start hidden load when first chosen; the window keeps the one it shows
      // until the new one's picture is there, rather than going blank.
      if (img.complete) swap();
      // A picture that never comes gets three seconds, so the tab and the window can't disagree for long.
      else {
        img.loading = 'eager';
        Promise.race([img.decode().catch(() => {}), new Promise(r => setTimeout(r, 3000))]).then(swap);
      }
    }
    // Pointing at the strip, or tabbing into it, fetches the other screens ahead of the choice.
    const warm = () => tabs.forEach(t => { panes[t.dataset.screen].querySelector('img').loading = 'eager'; });
    strip.addEventListener('pointerenter', warm, { once: true });
    strip.addEventListener('focusin', warm, { once: true });
    tabs.forEach((t, i) => {
      t.addEventListener('click', e => { e.preventDefault(); show(t.dataset.screen); });
      t.addEventListener('keydown', e => {
        const n = tabs.length;
        const to = { ArrowRight: i + 1, ArrowDown: i + 1, ArrowLeft: i - 1, ArrowUp: i - 1, Home: 0, End: n - 1 }[e.key];
        if (to === undefined) return;
        e.preventDefault();
        show(tabs[(to + n) % n].dataset.screen, true);
      });
    });

    /* ---------- The hero mascot (geometry and motion from the app's loader prototype) ---------- */
    const BODY = '#c396e7', EAR_INNER = '#8a63a9';
    const HEAD = [[350,470],[372,360],[430,262],[505,205],[570,160],[650,122],[730,94],[850,70],[970,64],[1090,64],[1170,73],[1250,86],[1330,104],[1400,170],[1470,262],[1495,350],[1513,410],[1549,475],[1640,550],[1725,600],[1783,650],[1834,725],[1855,825],[1851,875],[1830,925],[1782,975],[1678,1025],[1575,1052],[1489,1072],[1330,1083],[1090,1093],[890,1091],[650,1080],[410,1053],[290,1028],[210,997],[146,950],[116,900],[111,850],[117,800],[134,750],[171,700],[262,650],[320,600],[341,550],[348,500]];
    const EARS = [
      { outer: { up: [[312,190],[522,192],[580,178],[515,420],[362,462],[334,330]], flick: [[268,350],[522,192],[580,178],[515,420],[362,462],[305,420]] },
        inner: { up: [[356,256],[458,275],[406,405]], flick: [[332,352],[466,284],[402,420]] } },
      { outer: { up: [[1275,108],[1335,118],[1495,66],[1506,190],[1505,385],[1505,430],[1390,330]], flick: [[1275,108],[1335,118],[1612,250],[1560,325],[1515,395],[1505,430],[1390,330]] },
        inner: { up: [[1337,172],[1447,130],[1420,275]], flick: [[1345,182],[1545,242],[1440,300]] } },
    ];
    const TENTS = [
      { pts: [[487,1000],[480,1150],[472,1290],[450,1385],[400,1452],[320,1500],[220,1517],[128,1507]], phase: 0.0 },
      { pts: [[752,1000],[756,1200],[764,1450],[780,1640],[778,1730],[772,1795]], phase: 0.8 },
      { pts: [[1012,1000],[1022,1200],[1050,1460],[1085,1620],[1120,1715],[1143,1770]], phase: 1.6 },
      { pts: [[1227,1000],[1245,1200],[1295,1360],[1370,1500],[1460,1620],[1585,1745]], phase: 2.4 },
      { pts: [[1427,1000],[1462,1200],[1500,1290],[1560,1350],[1660,1380],[1750,1365],[1815,1330],[1858,1285]], phase: 3.2 },
    ];
    const EYES = [{ cx: 686, cy: 621, r: 106 }, { cx: 1228, cy: 544, r: 111 }];
    const CLOSED_EYES = ['M572 647Q680 607 798 625', 'M1115 583Q1230 539 1342 514'];
    const MOUTH = 'M932 790L982 705Q994 690 1010 700L1078 755';
    const BLUSH = [{ cx: 626, cy: 801, rx: 155, ry: 95 }, { cx: 1338, cy: 681, rx: 145, ry: 88 }];
    const f = p => p[0].toFixed(1) + ',' + p[1].toFixed(1);
    const pts = a => a.map(f).join(' ');
    const lerp = (a, b, t) => a.map((p, i) => [p[0] + (b[i][0] - p[0]) * t, p[1] + (b[i][1] - p[1]) * t]);
    function smooth(p, closed = false) {
      const n = p.length, at = i => closed ? p[(i + n) % n] : p[Math.max(0, Math.min(n - 1, i))];
      let d = 'M' + f(p[0]);
      for (let i = 0; i < (closed ? n : n - 1); i++) {
        const p0 = at(i - 1), p1 = at(i), p2 = at(i + 1), p3 = at(i + 2);
        d += 'C' + f([p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6]) + ' ' +
          f([p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6]) + ' ' + f(p2);
      }
      return closed ? d + 'Z' : d;
    }
    const host = document.getElementById('mascot');
    host.removeAttribute('tabindex'); host.removeAttribute('aria-hidden');
    host.innerHTML = `<svg viewBox="-40 -110 2080 2080" aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id="jt" gradientUnits="userSpaceOnUse" x1="0" y1="1050" x2="0" y2="1780"><stop offset="0" stop-color="#bd97e7"/><stop offset="1" stop-color="#9a8ee9"/></linearGradient>
        <radialGradient id="jb"><stop offset="0" stop-color="#e08ce0" stop-opacity=".9"/><stop offset="1" stop-color="#e08ce0" stop-opacity="0"/></radialGradient>
      </defs>
      <g class="all">
        <g fill="none" stroke="url(#jt)" stroke-width="134" stroke-linecap="round" stroke-linejoin="round">${TENTS.map(t => `<path class="tent" d="${smooth(t.pts)}"/>`).join('')}</g>
        <g class="bodyg">
          <path fill="${BODY}" d="${smooth(HEAD, true)}"/>
          ${EARS.map(e => `<g><polygon class="eo" points="${pts(e.outer.up)}" fill="${BODY}" stroke="${BODY}" stroke-width="40" stroke-linejoin="round"/><polygon class="ei" points="${pts(e.inner.up)}" fill="${EAR_INNER}" stroke="${EAR_INNER}" stroke-width="30" stroke-linejoin="round"/></g>`).join('')}
          ${BLUSH.map(b => `<ellipse cx="${b.cx}" cy="${b.cy}" rx="${b.rx}" ry="${b.ry}" fill="url(#jb)"/>`).join('')}
          <g class="look">
            <g class="open" fill="#fff">${EYES.map(e => `<ellipse class="eye" cx="${e.cx}" cy="${e.cy}" rx="${e.r}" ry="${e.r}"/>`).join('')}</g>
            <g class="closed" fill="none" stroke="#fff" stroke-width="34" stroke-linecap="round" display="none">${CLOSED_EYES.map(d => `<path d="${d}"/>`).join('')}</g>
          </g>
          <path d="${MOUTH}" fill="none" stroke="#fff" stroke-width="26" stroke-linecap="round" stroke-linejoin="round"/>
        </g>
      </g>
    </svg>`;
    const svg = host.firstElementChild;
    const el = {
      all: svg.querySelector('.all'), body: svg.querySelector('.bodyg'), look: svg.querySelector('.look'),
      open: svg.querySelector('.open'), closed: svg.querySelector('.closed'),
      eyes: [...svg.querySelectorAll('.eye')], tents: [...svg.querySelectorAll('.tent')],
      eo: [...svg.querySelectorAll('.eo')], ei: [...svg.querySelectorAll('.ei')],
    };

    let look = [0, 0], lookTo = [0, 0], flick = [0, 0], flickAt = -9, flickEars = [0], blinkAt = 1.2, happyUntil = -1, now = 0;
    const PERIOD = 2.6, FLICK_LEN = 0.55, BLINK_LEN = 0.24;

    // The eyes point at a spot on the page: the chosen tab, or the pointer.
    function aimAt(x, y) {
      const r = svg.getBoundingClientRect();
      const ex = r.left + r.width * 0.46, ey = r.top + r.height * 0.33;
      const dx = x - ex, dy = y - ey, d = Math.hypot(dx, dy) || 1;
      const k = Math.min(1, d / 260);
      lookTo = [dx / d * 46 * k, dy / d * 38 * k];
    }
    function aimAtTab() {
      const r = tabs.find(t => t.dataset.screen === current).getBoundingClientRect();
      aimAt(r.left + r.width / 2, r.top + r.height / 2);
    }

    // How much it bobs and sways, 0 to 1: it eases to rest once nothing has happened for a while.
    let amp = 1;
    function pose(time, still) {
      const ph = time * Math.PI * 2 / PERIOD, s = Math.sin(ph);
      const m = still ? 0 : amp;
      el.all.setAttribute('transform', `translate(0 ${(-30 * Math.sin(ph - 0.6) * m).toFixed(2)})`);
      el.body.setAttribute('transform', `translate(980 600) scale(${(1 - 0.03 * s * m).toFixed(4)} ${(1 + 0.03 * s * m).toFixed(4)}) translate(-980 -600)`);
      el.tents.forEach((p, i) => {
        const t = TENTS[i], n = t.pts.length;
        p.setAttribute('d', smooth(t.pts.map((q, j) => {
          if (!m) return q;
          const a = t.pts[Math.max(j - 1, 0)], b = t.pts[Math.min(j + 1, n - 1)];
          let tx = b[0] - a[0], ty = b[1] - a[1]; const len = Math.hypot(tx, ty) || 1; tx /= len; ty /= len;
          const u = j / (n - 1), w = 70 * Math.pow(u, 1.3) * Math.sin(ph - u * 2.4 + t.phase);
          return [q[0] - ty * w, q[1] + tx * w];
        })));
      });
      [0, 1].forEach(i => {
        let fl = 0;
        if (flickEars.includes(i)) {
          const u = (time - flickAt) / FLICK_LEN;
          if (u >= 0 && u <= 1) fl = Math.sin(u * Math.PI * 2) ** 2 * (u < 0.5 ? 1 : 0.55);
        }
        if (still) fl = flick[i];
        el.eo[i].setAttribute('points', pts(lerp(EARS[i].outer.up, EARS[i].outer.flick, fl)));
        el.ei[i].setAttribute('points', pts(lerp(EARS[i].inner.up, EARS[i].inner.flick, fl)));
      });
      // Eyes: shut after a pet, a blink now and then, else open and looking.
      let eye = 1;
      if (time < happyUntil) eye = 'closed';
      else if (!still) {
        const u = (time - blinkAt) / BLINK_LEN;
        if (u >= 0 && u <= 1) eye = u < .25 ? 1 - u / .25 * .65 : u < .7 ? 'closed' : .35 + (u - .7) / .3 * .65;
      }
      const shut = eye === 'closed';
      el.open.setAttribute('display', shut ? 'none' : 'inline');
      el.closed.setAttribute('display', shut ? 'inline' : 'none');
      el.eyes.forEach((e, i) => e.setAttribute('ry', (EYES[i].r * (shut ? 1 : eye)).toFixed(1)));
      look = still ? lookTo : [look[0] + (lookTo[0] - look[0]) * .18, look[1] + (lookTo[1] - look[1]) * .18];
      el.look.setAttribute('transform', `translate(${look[0].toFixed(1)} ${look[1].toFixed(1)})`);
    }

    const jelly = {
      // A new screen: look at its tab, and flick an ear at it.
      react() {
        aimAtTab();
        wake();
        flickAt = now; flickEars = [current === 'home' || current === 'library' ? 0 : 1];
        if (reduce) { flick = [0, 0]; flick[flickEars[0]] = .6; draw(); setTimeout(() => { flick = [0, 0]; draw(); }, 500); }
      },
    };
    function draw() { pose(now, true); }
    // Set below, where motion is allowed: starts the mascot moving again for a few seconds.
    let wake = () => {};

    host.addEventListener('click', () => {
      wake();
      happyUntil = now + 0.9; flickAt = now; flickEars = [0, 1];
      if (reduce) { happyUntil = Infinity; flick = [.6, .6]; draw(); setTimeout(() => { happyUntil = -1; flick = [0, 0]; draw(); }, 900); }
    });

    if (reduce) {
      show('home');
      addEventListener('resize', () => { aimAtTab(); draw(); });
      draw();
    } else {
      // It's awake for AWAKE seconds after the page opens, a screen is chosen, it's clicked or
      // the mouse moves, then eases to rest and the frames stop until the next of those.
      const AWAKE = 4, EASE = 0.9;
      const start = performance.now();
      const clock = () => (performance.now() - start) / 1000;
      let pointerAt = 0, awakeUntil = AWAKE, running = false, last = 0;
      function frame() {
        const t = clock(), dt = Math.min(t - last, 0.1);
        last = now = t;
        const awake = now < awakeUntil;
        amp = Math.max(0, Math.min(1, amp + (awake ? dt : -dt) / EASE));
        if (awake && now > blinkAt + BLINK_LEN) blinkAt = now + 2 + Math.random() * 3;
        if (now - pointerAt > 3 && pointerAt) { aimAtTab(); pointerAt = 0; }
        pose(now, false);
        const settling = now < blinkAt + BLINK_LEN && now > blinkAt || now < flickAt + FLICK_LEN || now < happyUntil
          || Math.hypot(look[0] - lookTo[0], look[1] - lookTo[1]) > 0.5;
        running = awake || amp > 0 || settling;
        if (running) requestAnimationFrame(frame);
      }
      wake = () => {
        now = clock();
        awakeUntil = now + AWAKE;
        if (!running) { running = true; last = now; requestAnimationFrame(frame); }
      };
      addEventListener('pointermove', e => {
        if (e.pointerType !== 'mouse') return;
        aimAt(e.clientX, e.clientY); wake(); pointerAt = now;
      }, { passive: true });
      show('home');
    }
  });
})();
