--!strict
-- StudioLine local plugin. The installer replaces the key when installing the plugin.
local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")
local MarketplaceService = game:GetService("MarketplaceService")

local LOCAL_KEY = "__STUDIOLINE_PLUGIN_KEY__"
local SOCKET_URL = "ws://127.0.0.1:38741/api/studio/socket"
local toolbar = plugin:CreateToolbar("StudioLine")
local toolbarButton = toolbar:CreateButton("StudioLine", "Open StudioLine connection", "rbxasset://StudioLine-logo.png", "StudioLine")
toolbarButton.ClickableWhenViewportHidden = true
local widget = plugin:CreateDockWidgetPluginGui(
    "StudioLineConnection",
    DockWidgetPluginGuiInfo.new(Enum.InitialDockState.Right, true, false, 300, 180, 230, 150)
)
widget.Title = "StudioLine"
local frame = Instance.new("Frame")
frame.BackgroundColor3 = Color3.fromRGB(26, 30, 26)
frame.BorderSizePixel = 0
frame.Size = UDim2.fromScale(1, 1)
frame.Parent = widget
-- Draw the selected logo locally, so the panel does not depend on a hosted image.
local logo = Instance.new("Frame")
logo.BackgroundColor3 = Color3.fromRGB(255, 255, 255)
logo.BorderSizePixel = 0
logo.Position = UDim2.fromOffset(16, 12)
logo.Size = UDim2.fromOffset(26, 26)
logo.Parent = frame
local function logoRect(x: number, y: number, width: number, height: number, color: Color3)
    local part = Instance.new("Frame")
    part.BorderSizePixel = 0
    part.BackgroundColor3 = color
    part.Position = UDim2.fromScale(x / 1024, y / 1024)
    part.Size = UDim2.fromScale(width / 1024, height / 1024)
    part.Parent = logo
end
local dark = Color3.fromRGB(32, 43, 40)
local green = Color3.fromRGB(91, 128, 107)
logoRect(230, 276, 338, 88, dark)
logoRect(230, 364, 88, 192, dark)
logoRect(318, 468, 235, 88, dark)
logoRect(465, 556, 88, 192, dark)
logoRect(230, 660, 235, 88, dark)
logoRect(624, 276, 88, 472, green)
logoRect(712, 660, 82, 88, green)
local title = Instance.new("TextLabel")
title.BackgroundTransparency = 1
title.Position = UDim2.fromOffset(52, 12)
title.Size = UDim2.new(1, -68, 0, 26)
title.Text = "Studio connection"
title.Font = Enum.Font.SourceSansSemibold
title.TextSize = 19
title.TextColor3 = Color3.fromRGB(230, 235, 228)
title.TextXAlignment = Enum.TextXAlignment.Left
title.Parent = frame
local status = Instance.new("TextLabel")
status.BackgroundTransparency = 1
status.Position = UDim2.fromOffset(16, 44)
status.Size = UDim2.new(1, -32, 0, 48)
status.Text = "Open a StudioLine project, then connect."
status.TextWrapped = true
status.Font = Enum.Font.SourceSans
status.TextSize = 15
status.TextColor3 = Color3.fromRGB(168, 179, 165)
status.TextXAlignment = Enum.TextXAlignment.Left
status.TextYAlignment = Enum.TextYAlignment.Top
status.Parent = frame
local button = Instance.new("TextButton")
button.Position = UDim2.fromOffset(16, 102)
button.Size = UDim2.new(1, -32, 0, 34)
button.BackgroundColor3 = Color3.fromRGB(190, 219, 177)
button.BorderSizePixel = 0
button.Text = "Connect"
button.Font = Enum.Font.SourceSansSemibold
button.TextSize = 16
button.TextColor3 = Color3.fromRGB(25, 35, 23)
button.Parent = frame
local corner = Instance.new("UICorner")
corner.CornerRadius = UDim.new(0, 5)
corner.Parent = button

toolbarButton.Click:Connect(function() widget.Enabled = not widget.Enabled end)

local current: WebStreamClient? = nil
local desired = false
local generation = 0
local retryCount = 0
local publishedPlaceName: string? = nil
local connections: {RBXScriptConnection} = {}

local function clearConnections()
    for _, connection in connections do connection:Disconnect() end
    table.clear(connections)
end

local function clipName(value: string): string
    local cut = utf8.offset(value, 121)
    if cut then return string.sub(value, 1, cut - 1) end
    return value
end

local function send(client: WebStreamClient, value: {[string]: any}): boolean
    local ok = pcall(function() client:Send(HttpService:JSONEncode(value)) end)
    return ok
end

local function inspectPlace(): {[string]: any}
    local children = workspace:GetChildren()
    local items = {}
    for i = 1, math.min(#children, 25) do
        local child = children[i]
        table.insert(items, {name = clipName(child.Name), class_name = child.ClassName})
    end
    return {name = publishedPlaceName or clipName(game.Name), place_id = game.PlaceId, workspace_children = items, total_children = #children}
end

local startConnection: () -> ()
startConnection = function()
    if not desired then return end
    if not RunService:IsEdit() then
        desired = false
        status.Text = "Stop the playtest, then connect again."
        button.Text = "Connect"
        return
    end
    generation += 1
    publishedPlaceName = nil
    local attempt = generation
    clearConnections()
    status.Text = "Connecting to the local app..."
    button.Text = "Disconnect"
    local ok, result = pcall(function()
        return HttpService:CreateWebStreamClient(Enum.WebStreamClientType.WebSocket, {Url = SOCKET_URL})
    end)
    if not ok then
        desired = false
        button.Text = "Connect"
        status.Text = "Could not connect. Open StudioLine and check the plugin's network permission."
        return
    end
    local client = result :: WebStreamClient
    current = client
    local helloSent = false
    local finished = false
    local function connectionLost()
        if attempt ~= generation or finished then return end
        finished = true
        current = nil
        clearConnections()
        pcall(function() client:Close() end)
        if desired then
            retryCount += 1
            if retryCount > 2 then
                desired = false
                status.Text = "Can't reach StudioLine. Open the app, then click Connect."
                button.Text = "Connect"
                return
            end
            status.Text = "Disconnected. Trying again..."
            local delaySeconds = math.min(2 ^ math.min(retryCount, 4), 15)
            task.delay(delaySeconds, function()
                if desired and attempt == generation then startConnection() end
            end)
        else
            button.Text = "Connect"
        end
    end
    local function opened()
        if attempt ~= generation or finished or helloSent then return end
        helloSent = true
        if not send(client, {type = "hello", token = LOCAL_KEY, place_name = clipName(game.Name), place_id = game.PlaceId, game_id = game.GameId}) then
            connectionLost()
        end
    end
    table.insert(connections, client.Opened:Connect(opened))
    table.insert(connections, client.MessageReceived:Connect(function(message: string)
        if attempt ~= generation or finished then return end
        local decodedOk, decoded = pcall(function() return HttpService:JSONDecode(message) end)
        if not decodedOk or type(decoded) ~= "table" then return end
        if decoded.type == "connected" then
            retryCount = 0
            status.Text = "Connected to StudioLine."
            -- game.Name is often a Studio-generated name such as Place2. Look up the
            -- published place name without delaying the connection or inspection.
            if game.PlaceId > 0 then
                task.spawn(function()
                    local okName, info = pcall(function()
                        return MarketplaceService:GetProductInfoAsync(game.PlaceId, Enum.InfoType.Asset)
                    end)
                    if attempt ~= generation or finished or not desired then return end
                    if okName and type(info) == "table" and info.AssetTypeId == 9 and type(info.Name) == "string" and info.Name ~= "" then
                        publishedPlaceName = clipName(info.Name)
                        send(client, {type = "place_name", name = publishedPlaceName})
                    end
                end)
            end
        elseif decoded.type == "shutdown" then
            desired = false
            status.Text = "StudioLine is closed. Open the app, then click Connect."
            button.Text = "Connect"
            send(client, {type = "shutdown_ack"})
            connectionLost()
        elseif decoded.type == "error" then
            desired = false
            status.Text = tostring(decoded.message or "Connection rejected. Open a project and try again.")
            button.Text = "Connect"
            connectionLost()
        elseif decoded.type == "command" and decoded.command == "inspect_place" and type(decoded.id) == "string" then
            local worked, data = false, nil
            if RunService:IsEdit() then worked, data = pcall(inspectPlace) end
            if worked then
                send(client, {type = "result", id = decoded.id, ok = true, data = data})
            else
                send(client, {type = "result", id = decoded.id, ok = false})
            end
        end
    end))
    table.insert(connections, client.Error:Connect(function(_code: number, _message: string)
        connectionLost()
    end))
    table.insert(connections, client.Closed:Connect(connectionLost))
    -- The handshake can finish before the event handler is connected. Send at most once.
    if client.ConnectionState == Enum.WebStreamClientState.Open then
        opened()
    elseif client.ConnectionState == Enum.WebStreamClientState.Closed or client.ConnectionState == Enum.WebStreamClientState.Error then
        connectionLost()
    end
    -- Recover if the engine never reports a handshake or error event.
    task.delay(8, function()
        if attempt == generation and not finished and not helloSent then connectionLost() end
    end)
end

button.MouseButton1Click:Connect(function()
    if desired then
        desired = false
        generation += 1
        local old = current
        current = nil
        clearConnections()
        if old then pcall(function() old:Close() end) end
        status.Text = "Disconnected."
        button.Text = "Connect"
    else
        desired = true
        retryCount = 0
        startConnection()
    end
end)
plugin.Unloading:Connect(function()
    desired = false
    generation += 1
    clearConnections()
    if current then pcall(function() (current :: WebStreamClient):Close() end) end
end)
