<p align="center">
  <img src="docs/assets/lotus.png" alt="Lotus icon" width="96" />
</p>

# lotus

A native Windows 11 dock, search, and Alt+Tab replacement.

<img src="docs/screenshots/dock.png" alt="Lotus dock" width="720" />

<img src="docs/screenshots/search.png" alt="Lotus application search" width="720" />

<img src="docs/screenshots/alt-tab.png" alt="Lotus Alt-Tab switcher" width="720" />

## install

Download the latest Lotus Setup from releases and run it. Lotus installs for your Windows
account, appears in the Start menu and Installed apps, and opens first setup on its first run.

The Windows zip remains available as a portable build: extract it and run `lotus.exe`. Portable
use does not require an installer; remove its extracted folder when you no longer need it.

## uninstall

Open Installed apps in Windows Settings and uninstall Lotus. For a portable copy, exit Lotus and
remove the extracted folder.

## support and recovery

Lotus supports native Windows 11. Some Windows shell flyout placement can vary by Windows build;
when Lotus cannot safely place a flyout, Windows keeps its normal placement.

If the dock or an integration is not behaving as expected, open Settings > About and try Restart
Lotus integration. Reset Lotus safely restores default settings while preserving a backup and
custom assets. Export diagnostics creates a local, redacted text report for support.

Lotus does not upload telemetry or diagnostics. The report is only written to the location you
choose and contains a safe settings summary, responsiveness metrics, and redacted recent local
diagnostic entries. It excludes saved application names, launch targets, custom paths, and the
full settings file.

Known limitations: Lotus does not replace every Windows shell surface, and Windows can deny or
supersede transient window activation while an app closes, opens, or changes focus. Those normal
races are reconciled quietly and recorded in local diagnostics.
