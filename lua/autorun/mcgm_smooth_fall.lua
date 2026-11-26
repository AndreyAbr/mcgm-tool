-- mcgm_smooth_fall.lua
-- Smooth step-down, block-descent, and landing smoothing on Minecraft maps.
-- Prediction-safe: never stalls falling velocity or skips client prediction ticks.

if SERVER then
    AddCSLuaFile()
end

local cvar_enabled = CreateConVar(
    "mcgm_smooth_fall",
    "1",
    bit.bor(FCVAR_ARCHIVE, FCVAR_REPLICATED),
    "Enable smooth step-down and landing smoothing in Minecraft maps"
)

local cvar_mc_phys = GetConVar("mcgm_physics_enabled")
local _cachedIsMcMap = nil

local function IsMinecraftMap()
    if _cachedIsMcMap ~= nil then
        return _cachedIsMcMap
    end
    local map = string.lower(game.GetMap() or "")
    local hit = false
    if _G.MCGM_PHYSICS and _G.MCGM_PHYSICS.Maps and _G.MCGM_PHYSICS.Maps[map] then
        hit = true
    elseif map:find("minecraft") ~= nil or map:find("^gm_mc") ~= nil or map:find("^mc_") ~= nil or map:find("^mcgm_") ~= nil then
        hit = true
    end
    if map ~= "" then
        _cachedIsMcMap = hit
    end
    return hit
end

local function ResetFallState(ply)
    ply.mcgm_sf_last_time = nil
    ply.mcgm_sf_was_grounded = nil
    ply.mcgm_sf_last_z = nil
    ply.mcgm_sf_last_vz = nil
end

local function IsSmoothFallAllowed()
    if not cvar_enabled:GetBool() then return false end
    if not cvar_mc_phys then
        cvar_mc_phys = GetConVar("mcgm_physics_enabled")
    end
    if cvar_mc_phys and cvar_mc_phys:GetInt() == 0 then return false end
    return true
end

hook.Add("FinishMove", "MCGM_SmoothBlockDescent", function(ply, mv)
    if not IsSmoothFallAllowed() then
        if IsValid(ply) then
            ResetFallState(ply)
            if CLIENT and ply == LocalPlayer() then
                ply.mcgm_sf_cam_z = 0
            end
        end
        return
    end
    if not IsValid(ply) or not ply:Alive() then return end
    if ply:GetMoveType() ~= MOVETYPE_WALK or ply:InVehicle() then
        ResetFallState(ply)
        return
    end
    if not IsMinecraftMap() then return end

    local cmdTime = CurTime()
    local onGround = ply:OnGround()
    local pos = mv:GetOrigin()
    local vel = mv:GetVelocity()

    -- Handle client prediction rollback cleanly without skipping FinishMove
    local prevTime = ply.mcgm_sf_last_time
    if prevTime ~= nil and cmdTime < prevTime then
        ply.mcgm_sf_was_grounded = onGround
        ply.mcgm_sf_last_z = pos.z
        ply.mcgm_sf_last_vz = vel.z
        ply.mcgm_sf_last_time = cmdTime
        return
    end

    local wasGrounded = ply.mcgm_sf_was_grounded
    local prevZ = ply.mcgm_sf_last_z
    local prevVZ = ply.mcgm_sf_last_vz or 0

    if wasGrounded == nil or prevZ == nil then
        ply.mcgm_sf_was_grounded = onGround
        ply.mcgm_sf_last_z = pos.z
        ply.mcgm_sf_last_vz = vel.z
        ply.mcgm_sf_last_time = cmdTime
        return
    end

    -- Never stall vel.z while airborne (preserves true gravity & jump arcs).
    -- Apply only a single-tick predicted landing bounce clamp when touching down.
    if onGround and not wasGrounded then
        if prevVZ < -120 and vel.z > 0 then
            vel.z = 0
            mv:SetVelocity(vel)
        end
    end

    -- Client-side visual landing camera cushion (strictly first-time predicted)
    if CLIENT and IsFirstTimePredicted() and ply == LocalPlayer() then
        local offset = ply.mcgm_sf_cam_z or 0
        if onGround and not wasGrounded and prevVZ < -150.0 then
            local cushion = math.Clamp(math.abs(prevVZ) * 0.015, 1.5, 8.0)
            offset = math.Clamp(offset + cushion, -24.0, 24.0)
        end
        ply.mcgm_sf_cam_z = offset
    end

    if cmdTime ~= prevTime then
        ply.mcgm_sf_was_grounded = onGround
        ply.mcgm_sf_last_z = pos.z
        ply.mcgm_sf_last_vz = vel.z
        ply.mcgm_sf_last_time = cmdTime
    end
end)

if CLIENT then
    hook.Add("CalcView", "MCGM_SmoothBlockCamera", function(ply, origin, angles, fov, znear, zfar)
        if not IsSmoothFallAllowed() then return end
        if not IsValid(ply) or ply ~= LocalPlayer() or not ply:Alive() then return end
        if ply:GetMoveType() ~= MOVETYPE_WALK or ply:InVehicle() then return end
        if ply:ShouldDrawLocalPlayer() then return end
        if not IsMinecraftMap() then return end

        local offset = ply.mcgm_sf_cam_z or 0
        if math.abs(offset) < 0.05 then
            ply.mcgm_sf_cam_z = 0
            return
        end

        local dt = math.Clamp(RealFrameTime(), 0, 0.05)
        offset = offset * math.exp(-22.0 * dt)
        if math.abs(offset) < 0.05 then
            ply.mcgm_sf_cam_z = 0
            return
        end
        ply.mcgm_sf_cam_z = offset

        return {
            origin = Vector(origin.x, origin.y, origin.z + offset),
            angles = angles,
            fov = fov,
            znear = znear,
            zfar = zfar,
            drawviewer = false
        }
    end)
end
