-- Runs immediately after the plugin source, sharing the mock locals.
assert(logo.ClassName == 'Frame' and logo.Parent == frame, 'The plugin panel should show the StudioLine logo')
assert(button.Text == 'Connect')
button.MouseButton1Click:Fire()
assert(#sockets == 1)
local first = sockets[1]
first.Opened:Fire()
assert(first.sent[1].type == 'hello')
assert(first.sent[1].place_name == 'Mock Place' and first.sent[1].game_id == 456)
first.MessageReceived:Fire({type = 'connected'})
assert(status.Text == 'Connected to StudioLine.')
assert(#spawned == 1)
spawned[1]()
assert(first.sent[2].type == 'place_name' and first.sent[2].name == 'Published place')
first.MessageReceived:Fire({type = 'command', command = 'inspect_place', id = 'request-1'})
local reply = first.sent[3]
assert(reply.type == 'result' and reply.id == 'request-1' and reply.ok)
assert(reply.data.name == 'Published place' and reply.data.place_id == 123)
assert(#reply.data.workspace_children == 25 and reply.data.total_children == 28)
button.MouseButton1Click:Fire()
assert(first.closed and button.Text == 'Connect' and status.Text == 'Disconnected.')

button.MouseButton1Click:Fire()
local second = sockets[2]
second.Opened:Fire()
second.Closed:Fire()
assert(status.Text == 'Disconnected. Trying again...')
local retry = scheduled[#scheduled]
button.MouseButton1Click:Fire()
retry()
assert(#sockets == 2, 'Cancelled reconnect must not create a new socket')

editMode = false
button.MouseButton1Click:Fire()
assert(button.Text == 'Connect' and status.Text == 'Stop the playtest, then connect again.')
editMode = true
button.MouseButton1Click:Fire()
local third = sockets[3]
third.Opened:Fire()
third.MessageReceived:Fire({type = 'error', message = 'Open a project first.'})
assert(status.Text == 'Open a project first.' and button.Text == 'Connect')
button.MouseButton1Click:Fire()
local fourth = sockets[4]
fourth.Error:Fire(0, 'connection refused')
assert(status.Text == 'Disconnected. Trying again...')
local errorRetry = scheduled[#scheduled]
button.MouseButton1Click:Fire()
errorRetry()
assert(#sockets == 4, 'Error retry must be cancellable')

button.MouseButton1Click:Fire()
local fifth = sockets[5]
local timeout = scheduled[#scheduled]
timeout()
assert(fifth.closed and status.Text == 'Disconnected. Trying again...')
button.MouseButton1Click:Fire()
-- An unavailable metadata lookup falls back to the Studio name and does not disconnect.
marketplaceFail = true
button.MouseButton1Click:Fire()
local offline = sockets[#sockets]
offline.Opened:Fire()
offline.MessageReceived:Fire({type = 'connected'})
spawned[#spawned]()
offline.MessageReceived:Fire({type = 'command', command = 'inspect_place', id = 'fallback'})
assert(offline.sent[2].data.name == 'Mock Place' and status.Text == 'Connected to StudioLine.')
button.MouseButton1Click:Fire()
marketplaceFail = false
nextSocketState = Enum.WebStreamClientState.Open
button.MouseButton1Click:Fire()
local sixth = sockets[#sockets]
assert(#sixth.sent == 1 and sixth.sent[1].type == 'hello', 'Already-open connection must send hello')
sixth.Opened:Fire()
assert(#sixth.sent == 1, 'Hello must not be sent twice')
sixth.MessageReceived:Fire({type = 'connected'})
sixth.MessageReceived:Fire({type = 'shutdown'})
assert(sixth.sent[2].type == 'shutdown_ack' and sixth.closed)
assert(status.Text == 'StudioLine is closed. Open the app, then click Connect.' and button.Text == 'Connect')
for _, callback in scheduled do callback() end
assert(#sockets == 7, 'Shutdown must not retry')

nextSocketState = Enum.WebStreamClientState.Connecting
button.MouseButton1Click:Fire()
for i = 1, 3 do
    local socket = sockets[#sockets]
    socket.Error:Fire(0, 'connection refused')
    if i < 3 then scheduled[#scheduled]() end
end
assert(button.Text == 'Connect' and status.Text == "Can't reach StudioLine. Open the app, then click Connect.")
local count = #sockets
for _, callback in scheduled do callback() end
assert(#sockets == count, 'Failed connections must not retry indefinitely')
plugin.Unloading:Fire()
print('Studio plugin mock behavior passed')
