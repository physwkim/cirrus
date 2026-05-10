-- 2D grid_scan against the MovingDot motors (mini:dot:mtrx,
-- mini:dot:mtry) using mini:current as the readback. The current
-- varies with time so each grid cell records a different value —
-- exercises the multi-axis Plan dispatch path.
--
-- Pre-req: VELO bumped on both motors (mini-beamline ships with
-- 0.2 unit/s default which overruns the 30s WRITE_NOTIFY timeout).
-- The script bumps them itself via short reads + an ad-hoc put.

print("[grid_scan] connecting to dot motors + beam current...")
local mx = ca_motor("dot_mtrx", "mini:dot:mtrx.VAL", "mini:dot:mtrx.RBV")
local my = ca_motor("dot_mtry", "mini:dot:mtry.VAL", "mini:dot:mtry.RBV")
local d  = ca_detector("beam_current", "mini:current")

print("[grid_scan] mtrx pos:", mx:locate().readback)
print("[grid_scan] mtry pos:", my:locate().readback)

-- 3x3 grid: 9 detector reads at 9 distinct (x,y) points.
print("[grid_scan] running 3x3 grid_scan...")
local result = RE:run(bp.grid_scan({d}, {
    {motor = mx, start = -1, stop = 1, num = 3},
    {motor = my, start = -1, stop = 1, num = 3},
}))
print("[grid_scan] result:", result)

assert(string.find(result, "exit_status=success", 1, true) ~= nil,
       "grid_scan failed: " .. result)
print("[grid_scan] OK")
