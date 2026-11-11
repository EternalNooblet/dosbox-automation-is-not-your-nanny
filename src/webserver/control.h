// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//

// Changed from upstream dosbox-automation in this fork (this repo is a fork, not the original upstream project).
// Last change to this file: 2026-11-01. Fork base commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#ifndef DOSBOX_WEBSERVER_CONTROL_H
#define DOSBOX_WEBSERVER_CONTROL_H

#include "private/shutdown.h"

#include "http/http.h"

namespace Webserver {

struct ControlHandlers {
	static void GetProgramState(const httplib::Request& req, httplib::Response& res);
	static void GetStatus(const httplib::Request& req, httplib::Response& res);
	static void PostPause(const httplib::Request& req, httplib::Response& res);
	static void PostResume(const httplib::Request& req, httplib::Response& res);
	static void GetPaused(const httplib::Request& req, httplib::Response& res);
};

} // namespace Webserver

#endif // DOSBOX_WEBSERVER_CONTROL_H
