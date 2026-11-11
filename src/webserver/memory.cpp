// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//

// Changed from upstream dosbox-automation in this fork (this repo is a fork, not the original upstream project).
// Last change to this file: 2026-11-02. Fork base commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#include "webserver.h"
#include "bridge.h"
#include "private/memory.h"

#include "base64/base64.h"
#include "http/http.h"
#include "nlohmann/json.hpp"
#include "utils/string_utils.h"

#include "cpu/registers.h"
#include "cpu/paging.h"
#include "dos/dos_memory.h"
#include "hardware/memory.h"

using json = nlohmann::json;
using httplib::Request, httplib::Response;

namespace Webserver {

static Segment str_to_base_segment(const std::string_view str)
{
	auto val = upcase(str);
	static const std::unordered_map<std::string_view, Segment> lookup = {
	        {"CS", Segment::CS},
	        {"SS", Segment::SS},
	        {"DS", Segment::DS},
	        {"ES", Segment::ES},
	        {"FS", Segment::FS},
	        {"GS", Segment::GS},
	};

	if (auto it = lookup.find(val); it != lookup.end()) {
		return it->second;
	} else {
		return Segment::None;
	}
}

static uint32_t base_segment_to_offset(const Segment segment)
{
	switch (segment) {
	case Segment::CS: return SegPhys(SegNames::cs);
	case Segment::SS: return SegPhys(SegNames::ss);
	case Segment::DS: return SegPhys(SegNames::ds);
	case Segment::ES: return SegPhys(SegNames::es);
	case Segment::FS: return SegPhys(SegNames::fs);
	case Segment::GS: return SegPhys(SegNames::gs);
	case Segment::None: return 0;
	default: return 0;
	}
}

static void parse_mem_addr(const httplib::Request& req, Segment& segment,
                           uint32_t& offset)
{
	offset  = num_param<uint32_t>(req, Source::Path, "offset");
	segment = Segment::None;

	if (req.path_params.find("segment") != req.path_params.end()) {
		auto& segment_param = req.path_params.at("segment");
		segment             = str_to_base_segment(segment_param);

		// Segment can either be a register to resolve later or an
		// address which we can already resolve here.
		if (segment == Segment::None) {
			const auto seg_addr = PhysicalMake(
			        num_param<uint16_t>(req, Source::Path, "segment"), 0);

			offset += seg_addr;
		}
	}
}

// True when every 4 KB page covered by [start, start+size) maps to plain
// host RAM (or ROM for reads). Without this, a span inside total RAM but
// beyond mapped pages faults into the illegal-page handler mid-scan and
// aborts the engine.
static bool range_mapped(uint64_t start, uint64_t size, const bool write)
{
	if (size == 0) {
		return true;
	}
	const uint64_t first_page = start / MemPageSize;
	const uint64_t last_page  = (start + size - 1) / MemPageSize;
	for (uint64_t page = first_page; page <= last_page; ++page) {
		const bool ok = write ? MEM_PageWritable(static_cast<Bitu>(page))
		                      : MEM_PageReadable(static_cast<Bitu>(page));
		if (!ok) {
			return false;
		}
	}
	return true;
}

void ReadMemoryCommand::Execute()
{
	regs.load();
	effective_addr = base_segment_to_offset(base) + offset;

	LOG_DEBUG("API: ReadMemoryCommand(0x%06x, %d)", effective_addr, len);

	// Guest-linear mode: offset has bit 31 set. Translates through the
	// guest's ACTIVE page directory (VCPI/EMS remaps included) instead of
	// reading host-physical identity pages, which hold stale/zero bytes
	// behind remaps. Physical reads never use this flag (host RAM tops at
	// 16 MB < 0x80000000).
	if (effective_addr & (1u << 31)) {
		uint32_t lin      = static_cast<uint32_t>(effective_addr & ~(1u << 31));
		uint64_t remaining = len;
		memory.resize(len);
		auto out = memory.data();
		while (remaining) {
			HostPt host;
			if (!PAGING_TranslateLinearRead(lin, host)) {
				memory.clear();
				error = "guest linear address 0x" + std::to_string(lin) +
				        " is not mapped or not readable";
				return;
			}
			uint64_t chunk = DosPageSize - (lin & (DosPageSize - 1));
			if (chunk > remaining) {
				chunk = remaining;
			}
			std::memcpy(out, host, static_cast<size_t>(chunk));
			out       += chunk;
			lin       += static_cast<uint32_t>(chunk);
			remaining -= chunk;
		}
		return;
	}

	const uint64_t mem_total = static_cast<uint64_t>(MEM_TotalPages()) *
	                           MemPageSize;

	const uint64_t end_addr = static_cast<uint64_t>(effective_addr) + len;

	if (end_addr > mem_total) {
		error = "Address range 0x" + std::to_string(effective_addr) + " + " +
		        std::to_string(len) + " exceeds emulated memory size (" +
		        std::to_string(mem_total) + " bytes)";
		return;
	}

	if (!range_mapped(effective_addr, len, false)) {
		error = "Address range 0x" + std::to_string(effective_addr) + " + " +
		        std::to_string(len) +
		        " includes an unmapped or device page";
		return;
	}

	memory.resize(len);
	if (!MEM_BlockReadHost(effective_addr, memory.data(), len)) {
		// A page's handler identity changed between the pre-check and the
		// copy (guest remap): reject instead of dispatching handler logic.
		memory.clear();
		error = "Address range 0x" + std::to_string(effective_addr) + " + " +
		        std::to_string(len) +
		        " includes a page remapped mid-read";
	}
}

void ReadMemoryCommand::Get(const Request& req, Response& res)
{
	// 128 MiB per request ought to be enough for everyone.
	// This limit just prevents bad things when accidentally requesting an
	// unreasonably large size.
	auto num_bytes = num_param<uint32_t>(req, Source::Path, "len", 1, 128 * 1024 * 1024);

	Segment segment;
	uint32_t offset;
	parse_mem_addr(req, segment, offset);

	ReadMemoryCommand cmd(segment, offset, num_bytes);
	cmd.WaitForCompletion();

	if (!cmd.error.empty()) {
		throw std::out_of_range(cmd.error);
	}

	if (req.get_header_value("accept").starts_with(TypeJson)) {
		json j;
		j["registers"]      = cmd.regs;
		j["memory"]["addr"] = cmd.effective_addr;
		j["memory"]["data"] = base64::to_base64(cmd.memory);

		send_json(res, j);

	} else {
		// Only do base64 if explicitly requested, binary/download by
		// default.
		res.set_header("Content-Disposition",
		               "attachment; filename =\"memory.bin\"");
		res.set_content(cmd.memory, TypeBinary);
	}
}

void WriteMemoryCommand::Execute()
{
	effective_addr = base_segment_to_offset(base) + offset;

	LOG_DEBUG("API: WriteMemoryCommand(0x%06x, %d)", effective_addr, data.size());

	const uint64_t mem_total = static_cast<uint64_t>(MEM_TotalPages()) *
	                           MemPageSize;

	const uint64_t end_addr = static_cast<uint64_t>(effective_addr) +
	                          data.size();

	if (end_addr > mem_total) {
		error = "Address range 0x" + std::to_string(effective_addr) +
		        " + " + std::to_string(data.size()) +
		        " exceeds emulated memory size (" +
		        std::to_string(mem_total) + " bytes)";
		return;
	}

	if (!range_mapped(effective_addr, data.size(), true)) {
		error = "Address range 0x" + std::to_string(effective_addr) + " + " +
		        std::to_string(data.size()) +
		        " includes an unmapped, read-only or device page";
		return;
	}

	if (!expected_data.empty()) {
		conflict_data.resize(expected_data.size());

		MEM_BlockRead(effective_addr,
		              conflict_data.data(),
		              conflict_data.size());

		if (expected_data != conflict_data) {
			return;
		}

		conflict_data.clear();
	}

	MEM_BlockWrite(effective_addr, data.data(), data.size());
}

void WriteMemoryCommand::Put(const httplib::Request& req, httplib::Response& res)
{
	Segment segment;
	uint32_t offset;
	parse_mem_addr(req, segment, offset);

	constexpr size_t MaxWriteBytes = 128 * 1024 * 1024; // 128 MiB

	std::string data;

	if (req.get_header_value("Content-Type") == TypeJson) {
		auto j = json::parse(req.body);
		data   = base64::from_base64(j.at("data").get<std::string>());

	} else if (req.get_header_value("Content-Type") == TypeBinary) {
		data = req.body;

	} else {
		throw std::invalid_argument("Content-Type must be either " +
		                            std::string(TypeJson) + " or " +
		                            std::string(TypeBinary));
	}

	if (data.size() > MaxWriteBytes) {
		throw std::invalid_argument(
		        "Write data exceeds maximum size of 128 MiB");
	}

	std::string expected_data;

	if (req.has_header("If-Match")) {
		// The standard requires ETags in this and ETags are quoted
		// but we accept unquoted because no one is gonna bother
		std::string etag_hdr  = req.get_header_value("If-Match");
		std::string_view etag = etag_hdr;
		if (etag.starts_with('"') && etag.ends_with('"')) {
			etag.remove_prefix(1);
			etag.remove_suffix(1);
		}
		expected_data = base64::from_base64(etag);
	}

	WriteMemoryCommand cmd(segment, offset, std::move(data), std::move(expected_data));
	cmd.WaitForCompletion();

	if (!cmd.error.empty()) {
		throw std::out_of_range(cmd.error);
	}

	json j;
	j["memory"]["addr"] = cmd.effective_addr;
	if (!cmd.conflict_data.empty()) {
		res.status = httplib::StatusCode::PreconditionFailed_412;
		j["memory"]["data"] = base64::to_base64(cmd.conflict_data);
	}

	send_json(res, j);
}

// --- Memory value scan ---

std::vector<uint32_t> ScanBufferForValue(const std::vector<uint8_t>& buf,
                                         const uint32_t value, const int width)
{
	if (width != 1 && width != 2 && width != 4) {
		throw std::invalid_argument("width must be 1, 2, or 4");
	}
	std::vector<uint32_t> hits;
	if (buf.size() < static_cast<size_t>(width)) {
		return hits;
	}
	for (uint32_t i = 0; i + width <= buf.size(); ++i) {
		uint32_t v = 0;
		for (int b = 0; b < width; ++b) {
			v |= static_cast<uint32_t>(buf[i + b]) << (8 * b);
		}
		if (v == value) {
			hits.push_back(i);
		}
	}
	return hits;
}

void SearchMemoryCommand::Execute()
{
	const uint64_t mem_total = static_cast<uint64_t>(MEM_TotalPages()) *
	                           MemPageSize;
	if (end > mem_total || start >= end) {
		error = "Invalid search range";
		return;
	}
	const auto len = end - start;
	if (!range_mapped(start, len, false)) {
		error = "Search range 0x" + std::to_string(start) + "..0x" +
		        std::to_string(end) + " includes an unmapped or device page";
		return;
	}

	std::vector<uint8_t> buf(len);
	MEM_BlockRead(start, buf.data(), len);
	matches = ScanBufferForValue(buf, value, width);
	for (auto& m : matches) {
		m += start;
	}
}

void SearchMemoryCommand::Post(const Request& req, Response& res)
{
	auto j = json::parse(req.body);

	const uint32_t value = j.at("value").get<uint32_t>();
	const int width      = j.value("width", 1);
	const uint32_t start = j.at("start").get<uint32_t>();
	const uint32_t end   = j.at("end").get<uint32_t>();

	if (width != 1 && width != 2 && width != 4) {
		throw std::invalid_argument("width must be 1, 2, or 4");
	}

	constexpr uint32_t MaxSpan = 16 * 1024 * 1024;
	if (end <= start || end - start > MaxSpan) {
		throw std::invalid_argument("search span must be 1.." +
		                            std::to_string(MaxSpan) + " bytes");
	}

	SearchMemoryCommand cmd(start, end, value, width);
	cmd.WaitForCompletion(2000);
	if (!cmd.error.empty()) {
		throw std::out_of_range(cmd.error);
	}

	json result;
	result["matches"] = cmd.Matches();
	send_json(res, result);
}

void PageMapCommand::Execute()
{
	total_pages = static_cast<uint32_t>(MEM_TotalPages());
	readable.resize(total_pages);
	writable.resize(total_pages);
	for (uint32_t page = 0; page < total_pages; ++page) {
		readable[page] = MEM_PageReadable(page) ? 1 : 0;
		writable[page] = MEM_PageWritable(page) ? 1 : 0;
	}
}

void PageMapCommand::Get(const Request& req, Response& res)
{
	PageMapCommand cmd;
	cmd.WaitForCompletion();

	json j;
	j["total_pages"] = cmd.total_pages;
	j["page_size"]   = static_cast<uint32_t>(MemPageSize);
	// Plain '1'/'0' per page so a client can index directly by page.
	j["readable"] = std::string(cmd.readable.begin(), cmd.readable.end());
	j["writable"] = std::string(cmd.writable.begin(), cmd.writable.end());
	send_json(res, j);
}

} // namespace Webserver
