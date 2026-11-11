// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//
// Changed from upstream dosbox-automation in this fork (this repo is a fork, not
// the original upstream project). Last change to this file: 2026-11-11. Fork base
// commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#ifndef DOSBOX_SHELL_BANNER_H
#define DOSBOX_SHELL_BANNER_H

#include "dosbox.h"
#include "gui/mapper.h"

// Compact two-line startup banner, replaces the old welcome box.
// Shared between the shell message registration and the width tests;
// every rendered line must stay within ShellBannerMaxColumns
// (tests/shell_banner_tests.cpp enforces this, translations included).

constexpr int ShellBannerMaxColumns = 78;

// Format arguments: detailed version string, webserver status tag,
// workbench hint line (ShellBannerWorkbenchHint when the API is on,
// "" when it is off).
// [color=white] sets the ANSI bright attribute which [color=light-gray]
// alone does not clear; every bright-to-dim transition needs a [reset].
constexpr char ShellBannerFormat[] =
        "[color=yellow]dosbox-full-auto [color=white]%s[reset] %s\n"
        "[color=light-gray]Command list: [color=white]HELP[reset][color=light-gray],"
        " instructions: [color=white]MANUAL[reset]\n"
        "%s"
        "\n";

constexpr char ShellBannerWorkbenchHint[] =
        "[color=light-gray]Cheat Workbench: type [color=white]WORKBENCH[reset]"
        "[color=light-gray] or press [color=white]" PRIMARY_MOD_NAME
        "+" MMOD2_NAME "+W[reset]\n";

constexpr char ShellBannerWebserverEnabledTag[] = "[color=light-green][webserver enabled][reset]";

constexpr char ShellBannerWebserverDisabledTag[] = "[color=dark-gray][webserver disabled][reset]";

#endif // DOSBOX_SHELL_BANNER_H
