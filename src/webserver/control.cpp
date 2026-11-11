// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//

// Changed from upstream dosbox-automation in this fork (this repo is a fork, not the original upstream project).
// Last change to this file: 2026-11-01. Fork base commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#include "control.h"
#include "bridge.h"
#include "webserver.h"

#include "dos/archive/archive_activity.h"
#include "dosbox.h"
#include "gui/titlebar.h"
#include "shell/shell.h"

#include "nlohmann/json.hpp"

using json = nlohmann::json;

namespace Webserver {

void ShutdownCommand::Execute()
{
	DOSBOX_RequestShutdown();
}

void ShutdownCommand::Post(const httplib::Request&, httplib::Response& res)
{
	ShutdownCommand cmd;
	cmd.WaitForCompletion();

	json j;
	j["status"] = "shutdown_requested";
	send_json(res, j);
}

void ControlHandlers::PostPause(const httplib::Request&, httplib::Response& res)
{
	WEBSERVER_SetPaused(true);

	json j;
	j["paused"] = true;
	j["note"]   = "machine held at an instruction boundary; memory and "
	              "freeze commands answer while paused; resume with "
	              "/api/v1/control/resume";
	send_json(res, j);
}

void ControlHandlers::PostResume(const httplib::Request&, httplib::Response& res)
{
	WEBSERVER_SetPaused(false);

	json j;
	j["paused"] = false;
	send_json(res, j);
}

void ControlHandlers::GetPaused(const httplib::Request&, httplib::Response& res)
{
	json j;
	j["paused"] = WEBSERVER_IsPaused();
	send_json(res, j);
}

void ControlHandlers::GetProgramState(const httplib::Request&, httplib::Response& res)
{
	json j;
	j["segment_name"] = TITLEBAR_GetSegmentName();
	j["canonical_name"] = TITLEBAR_GetCanonicalName();
	j["is_shell"] = SHELL_IsRunning();
	j["is_booted"] = TITLEBAR_IsBooted();
	send_json(res, j);
}

void ControlHandlers::GetStatus(const httplib::Request&, httplib::Response& res)
{
	json j;
	j["running"] = true;
	j["shutdown_requested"] = DOSBOX_IsShutdownRequested();
	j["is_booted"] = TITLEBAR_IsBooted();
	j["program"] = TITLEBAR_GetSegmentName();
	j["canonical_name"] = TITLEBAR_GetCanonicalName();
	j["is_shell"] = SHELL_IsRunning();
	// Read here, not through the Bridge: a mount that waits on the archive
	// helper holds the emulation thread.
	if (const auto archive = ArchiveMount::Activity::CurrentStatus()) {
		j["archive"] = {
		        {"activity", archive->activity},
		        {"archive", archive->archive},
		        {"bytes_done", archive->bytes_done},
		        {"bytes_total", archive->bytes_total},
		};
	}
	send_json(res, j);
}

} // namespace Webserver
