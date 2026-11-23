-- Minimal Studio fixture. This is not a replacement for testing in Roblox Studio.
local function signal()
    local handlers = {}
    local event = {}
    function event:Connect(fn)
        local entry = {fn = fn, active = true}
        table.insert(handlers, entry)
        return {Disconnect = function() entry.active = false end}
    end
    function event:Fire(...)
        for _, entry in table.clone(handlers) do
            if entry.active then entry.fn(...) end
        end
    end
    return event
end
local Enum = {InitialDockState = {Right = 1}, Font = {SourceSansSemibold = 1, SourceSans = 2}, TextXAlignment = {Left = 1}, TextYAlignment = {Top = 1}, WebStreamClientType = {WebSocket = 1}, InfoType = {Asset = 1}, WebStreamClientState = {Connecting = 1, Open = 2, Error = 3, Closed = 4}}
local Color3 = {fromRGB = function(...) return {...} end}
local UDim2 = {fromScale = function(...) return {...} end, fromOffset = function(...) return {...} end, new = function(...) return {...} end}
local UDim = {new = function(...) return {...} end}
local DockWidgetPluginGuiInfo = {new = function(...) return {...} end}
local Instance = {new = function(className)
    local object = {ClassName = className}
    if className == 'TextButton' then object.MouseButton1Click = signal() end
    return object
end}
local scheduled = {}
local spawned = {}
local task = {delay = function(_, fn) table.insert(scheduled, fn) end, spawn = function(fn) table.insert(spawned, fn) end}
local sockets = {}
local nextSocketState = Enum.WebStreamClientState.Connecting
local HttpService = {}
function HttpService:CreateWebStreamClient(kind, options)
    assert(kind == Enum.WebStreamClientType.WebSocket)
    assert(options.Url == 'ws://127.0.0.1:38741/api/studio/socket')
    local socket = {ConnectionState = nextSocketState, Opened = signal(), MessageReceived = signal(), Error = signal(), Closed = signal(), sent = {}}
    function socket:Send(value) table.insert(self.sent, value) end
    function socket:Close() self.closed = true; self.Closed:Fire() end
    table.insert(sockets, socket)
    return socket
end
function HttpService:JSONEncode(value) return value end
function HttpService:JSONDecode(value) return value end
local editMode = true
local RunService = {IsEdit = function() return editMode end}
local marketplaceFail = false
local MarketplaceService = {GetProductInfoAsync = function(_, id, kind)
    assert(id == 123 and kind == Enum.InfoType.Asset)
    if marketplaceFail then error('not available') end
    return {Name = 'Published place', AssetTypeId = 9}
end}
local game = {Name = 'Mock Place', PlaceId = 123, GameId = 456}
function game:GetService(name)
    if name == 'HttpService' then return HttpService end
    if name == 'RunService' then return RunService end
    if name == 'MarketplaceService' then return MarketplaceService end
    error('unknown service')
end
local children = {}
for i = 1, 28 do table.insert(children, {Name = 'Part' .. i, ClassName = 'Part'}) end
local workspace = {GetChildren = function() return children end}
local widget = {Enabled = true}
local plugin = {Unloading = signal()}
function plugin:CreateToolbar(_)
    return {CreateButton = function(_, _, _, icon) assert(icon == "rbxasset://StudioLine-logo.png"); return {Click = signal()} end}
end
function plugin:CreateDockWidgetPluginGui(_, _) return widget end
