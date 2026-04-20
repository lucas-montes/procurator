# Worker — VM Management Daemon

It has 3 main parts.

## Server
A rpc server listening for commands to apply, it can create, delete, list and so on.

## Registry
The strucutre holding the state with the vms running. Split into two faces, the write and the read face.
The separation is to avoid locking. However it has an sqlite db so the reader face could have some 'quick' write capabilities?

## Supervisor
Holds the writting part of registry. It receives commands from a channel comming from the server part. Once a command comes in it executes it

## Factory
A structure that is dependand of the backend used, currently it's cloud hypervisor.
It creates the vms (in this case it spawns a process and creates a client to communicate with it)


## TODO
[ ] Need a better way to keep state and failures to clean up the directories
[ ] The values for the rpcs to return stats probably should be abstract as well
