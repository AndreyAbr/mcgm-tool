-- mcgm_parkour.lua
-- Map-scoped parkour checkpoints & finish markers from Minecraft signs.
-- Works independently of Minecraft movement physics settings (mcgm_physics_enabled).

if not SERVER then
    return
end

local MAP_DIR = "mcgm_parkour/"
local MCGM_PARKOUR = _G.MCGM_PARKOUR or {}
MCGM_PARKOUR.Maps = MCGM_PARKOUR.Maps or {}
MCGM_PARKOUR.ActiveMap = MCGM_PARKOUR.ActiveMap or nil
MCGM_PARKOUR.LoadedMapName = MCGM_PARKOUR.LoadedMapName or ""
MCGM_PARKOUR.PlayerState = MCGM_PARKOUR.PlayerState or {}
_G.MCGM_PARKOUR = MCGM_PARKOUR

local function ResetAllPlayerStates()
    MCGM_PARKOUR.PlayerState = {}
end

local function SyncLoadedMap()
    local curMap = string.lower(game.GetMap() or "")
    if curMap == "" then
        if MCGM_PARKOUR.LoadedMapName ~= "" then
            MCGM_PARKOUR.LoadedMapName = ""
            MCGM_PARKOUR.ActiveMap = nil
            ResetAllPlayerStates()
        end
        return nil
    end

    if MCGM_PARKOUR.LoadedMapName ~= curMap then
        MCGM_PARKOUR.LoadedMapName = curMap
        MCGM_PARKOUR.ActiveMap = nil
        ResetAllPlayerStates()
    end

    if MCGM_PARKOUR.ActiveMap then
        return MCGM_PARKOUR.ActiveMap
    end

    if MCGM_PARKOUR.Maps[curMap] then
        MCGM_PARKOUR.ActiveMap = MCGM_PARKOUR.Maps[curMap]
        return MCGM_PARKOUR.ActiveMap
    end

    local mapFile = MAP_DIR .. curMap .. ".lua"
    if file.Exists(mapFile, "LUA") then
        include(mapFile)
        if MCGM_PARKOUR.Maps[curMap] then
            MCGM_PARKOUR.ActiveMap = MCGM_PARKOUR.Maps[curMap]
            return MCGM_PARKOUR.ActiveMap
        end
    end

    return nil
end

function _G.MCGM_RegisterParkourMap(mapName, data)
    if not isstring(mapName) or not istable(data) then
        return
    end
    local key = string.lower(mapName)
    MCGM_PARKOUR.Maps[key] = data
    if string.lower(game.GetMap() or "") == key then
        if MCGM_PARKOUR.LoadedMapName ~= key then
            MCGM_PARKOUR.LoadedMapName = key
            ResetAllPlayerStates()
        end
        MCGM_PARKOUR.ActiveMap = data
    end
end

local function GetPlayerKey(ply)
    local sid = ply.SteamID64 and ply:SteamID64() or ""
    if isstring(sid) and sid ~= "" and sid ~= "0" then
        return sid
    end
    return "ent_" .. tostring(ply:EntIndex())
end

local function GetPlayerState(ply)
    local key = GetPlayerKey(ply)
    local st = MCGM_PARKOUR.PlayerState[key]
    if not st then
        st = {
            checkpoint = nil,
            finished = false,
            inside = {},
            warnedInvalid = {},
        }
        MCGM_PARKOUR.PlayerState[key] = st
    end
    return st
end

local function AABBOverlaps(aMin, aMax, bMin, bMax)
    return aMin[1] <= bMax[1] and aMax[1] >= bMin[1]
        and aMin[2] <= bMax[2] and aMax[2] >= bMin[2]
        and aMin[3] <= bMax[3] and aMax[3] >= bMin[3]
end

local function PlayerOverlapsMarker(ply, marker)
    local pos = ply:GetPos()
    local hMin, hMax = ply:GetHull()
    local mins = hMin or Vector(-16, -16, 0)
    local maxs = hMax or Vector(16, 16, 72)
    local pMin = { pos.x + mins.x, pos.y + mins.y, pos.z + mins.z }
    local pMax = { pos.x + maxs.x, pos.y + maxs.y, pos.z + maxs.z }
    return AABBOverlaps(pMin, pMax, marker.min, marker.max)
end

local function TryValidateGroundCandidate(ply, candidate, mins, maxs, scale)
    local startOffsets = { 0.25 * scale, 0.0 }
    local downDist = 2.6 * scale
    local traceFilter = function(ent)
        return not (IsValid(ent) and ent:IsPlayer())
    end

    for _, zOff in ipairs(startOffsets) do
        local startPos = Vector(candidate.x, candidate.y, candidate.z + zOff)
        local endPos = Vector(candidate.x, candidate.y, candidate.z - downDist)

        local trDown = util.TraceHull({
            start = startPos,
            endpos = endPos,
            mins = mins,
            maxs = maxs,
            filter = traceFilter,
            mask = MASK_PLAYERSOLID,
        })

        if trDown.Hit and not trDown.StartSolid and not trDown.AllSolid then
            if trDown.HitNormal and trDown.HitNormal.z >= 0.7 then
                local standPos = trDown.HitPos + Vector(0, 0, 1)
                local trStand = util.TraceHull({
                    start = standPos,
                    endpos = standPos,
                    mins = mins,
                    maxs = maxs,
                    filter = traceFilter,
                    mask = MASK_PLAYERSOLID,
                })
                if not trStand.StartSolid and not trStand.AllSolid and util.IsInWorld(standPos) then
                    local midPos = Vector(standPos.x, standPos.y, standPos.z + (maxs.z * 0.5))
                    if bit.band(util.PointContents(midPos), CONTENTS_SLIME) == 0 then
                        return standPos
                    end
                end
            end
        end
    end

    return nil
end

local function FindSafeCheckpointPos(ply, marker, scale)
    local S = (tonumber(scale) and tonumber(scale) > 0) and tonumber(scale) or 39.37
    local hMin, hMax = ply:GetHull()
    local mins = hMin or Vector(-16, -16, 0)
    local maxs = hMax or Vector(16, 16, 72)

    local a = marker.anchor
    local anchorVec = Vector(a[1], a[2], a[3])
    local f = marker.fwd or { 1, 0 }
    local fwd = Vector(f[1] or 1, f[2] or 0, 0)
    if fwd:LengthSqr() < 0.001 then
        fwd = Vector(1, 0, 0)
    else
        fwd:Normalize()
    end
    local right = Vector(fwd.y, -fwd.x, 0)

    local candidates = {
        anchorVec,
        anchorVec + fwd * (0.45 * S),
        anchorVec + fwd * (0.85 * S),
        anchorVec + right * (0.45 * S),
        anchorVec - right * (0.45 * S),
        anchorVec + fwd * (0.45 * S) + right * (0.45 * S),
        anchorVec + fwd * (0.45 * S) - right * (0.45 * S),
    }

    if not marker.is_wall then
        candidates[#candidates + 1] = anchorVec - fwd * (0.50 * S)
    end

    if ply:IsOnGround() then
        candidates[#candidates + 1] = ply:GetPos()
    end

    for _, cand in ipairs(candidates) do
        local safePos = TryValidateGroundCandidate(ply, cand, mins, maxs, S)
        if safePos then
            return safePos
        end
    end

    return nil
end

local function TeleportPlayerToCheckpoint(ply, cp)
    if not IsValid(ply) or not cp or not cp.pos then
        return
    end
    ply:SetPos(cp.pos)
    if cp.ang then
        ply:SetEyeAngles(cp.ang)
    end
    local vel = ply:GetVelocity()
    if vel and vel:LengthSqr() > 0.01 then
        ply:SetVelocity(-vel)
    end
end

local function HandleMarkerEnter(ply, marker, state, scale)
    if marker.kind == "checkpoint" then
        if state.checkpoint and state.checkpoint.id == marker.id then
            return
        end
        local safePos = FindSafeCheckpointPos(ply, marker, scale)
        if safePos then
            state.checkpoint = {
                id = marker.id,
                name = marker.name or "",
                pos = safePos,
                ang = Angle(0, marker.yaw or 0, 0),
            }
            state.finished = false
            state.warnedInvalid[marker.id] = nil
            local label = (marker.name and marker.name ~= "")
                and ("Контрольная точка: " .. marker.name)
                or "Контрольная точка"
            ply:ChatPrint("[MCGM] " .. label)
        elseif not state.warnedInvalid[marker.id] then
            state.warnedInvalid[marker.id] = true
            local mc = marker.mc or { 0, 0, 0 }
            local a = marker.anchor or { 0, 0, 0 }
            local nameTag = (marker.name and marker.name ~= "") and (" '" .. marker.name .. "'") or ""
            MsgN(string.format(
                "[MCGM Parkour] Warning: no safe respawn position near checkpoint%s (%s) at MC (%d, %d, %d) / Source (%.1f, %.1f, %.1f); keeping previous checkpoint.",
                nameTag,
                tostring(marker.id),
                mc[1] or 0,
                mc[2] or 0,
                mc[3] or 0,
                a[1] or 0,
                a[2] or 0,
                a[3] or 0
            ))
        end
    elseif marker.kind == "finish" then
        if not state.finished then
            state.checkpoint = nil
            state.finished = true
            local msg = (marker.name and marker.name ~= "")
                and ("Финиш: " .. marker.name .. "! Прохождение завершено.")
                or "Финиш! Прохождение завершено."
            ply:ChatPrint("[MCGM] " .. msg)
        end
    end
end

SyncLoadedMap()

hook.Add("InitPostEntity", "MCGM_ParkourInit", function()
    ResetAllPlayerStates()
    MCGM_PARKOUR.LoadedMapName = ""
    MCGM_PARKOUR.ActiveMap = nil
    SyncLoadedMap()
end)

hook.Add("ShutDown", "MCGM_ParkourShutdown", function()
    ResetAllPlayerStates()
    MCGM_PARKOUR.ActiveMap = nil
    MCGM_PARKOUR.LoadedMapName = ""
end)

hook.Add("PlayerDisconnected", "MCGM_ParkourCleanupPlayer", function(ply)
    if not IsValid(ply) then
        return
    end
    MCGM_PARKOUR.PlayerState[GetPlayerKey(ply)] = nil
end)

local nextScanTime = 0
hook.Add("Think", "MCGM_ParkourCheckMarkers", function()
    local now = CurTime()
    if now < nextScanTime then
        return
    end
    nextScanTime = now + 0.05

    local mapCfg = SyncLoadedMap()
    if not mapCfg or not istable(mapCfg.markers) or #mapCfg.markers == 0 then
        return
    end

    local scale = tonumber(mapCfg.scale) or 39.37
    for _, ply in ipairs(player.GetAll()) do
        if IsValid(ply) and ply:Alive() and not ply:InVehicle() then
            local state = GetPlayerState(ply)
            for _, marker in ipairs(mapCfg.markers) do
                local isIn = PlayerOverlapsMarker(ply, marker)
                local wasIn = state.inside[marker.id] == true
                if isIn and not wasIn then
                    state.inside[marker.id] = true
                    HandleMarkerEnter(ply, marker, state, scale)
                elseif not isIn and wasIn then
                    state.inside[marker.id] = nil
                    state.warnedInvalid[marker.id] = nil
                end
            end
        end
    end
end)

hook.Add("PlayerSpawn", "MCGM_ParkourRespawn", function(ply, transition)
    if transition or not IsValid(ply) then
        return
    end
    local mapCfg = SyncLoadedMap()
    if not mapCfg or not istable(mapCfg.markers) or #mapCfg.markers == 0 then
        return
    end

    local state = GetPlayerState(ply)
    local cp = state.checkpoint
    if not cp or not cp.pos then
        -- Normal spawn starts a new run when no active checkpoint is saved.
        state.finished = false
        return
    end

    TeleportPlayerToCheckpoint(ply, cp)
    timer.Simple(0, function()
        if IsValid(ply) and ply:Alive() and state.checkpoint == cp then
            TeleportPlayerToCheckpoint(ply, cp)
        end
    end)
end)
