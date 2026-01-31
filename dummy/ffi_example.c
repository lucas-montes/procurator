#include <stdio.h>
#include <stdlib.h>
#include "dummy_rust.h"

int main(int argc, char *argv[]) {
    printf("=== Rust FFI Example with Structs & Pointers ===\n\n");

    StateHandle *handle = peer_create();
    if (handle == NULL) {
        fprintf(stderr, "Failed to create peer handle.\n");
        return EXIT_FAILURE;
    }

    PeerConfig config = {
        .semaphore_endpoint = "ipc://example-semaphore",
        .panic_on_disconnection = true,
    };

    ErrorCode init_result = peer_init(handle, config);
    if (init_result.tag != Ok) {
        fprintf(stderr, "Failed to initialize peer. Error code: %d\n", init_result.tag);
        return EXIT_FAILURE;
    }


    ErrorCode result = peer_set_counter(handle, 42);
    if (result.tag != Ok) {
        fprintf(stderr, "Failed to set counter. Error code: %d\n", result.tag);
    }

    ErrorCode start_result = peer_start(handle);
    if (start_result.tag != Ok) {
        fprintf(stderr, "Failed to start peer. Error code: %d\n", start_result.tag);
        return EXIT_FAILURE;
    }

    ErrorCode stop_result = peer_set_counter(handle, 100);
    if (stop_result.tag != Ok) {
        fprintf(stderr, "Failed to set counter. Error code: %d\n", stop_result.tag);
    }

    ErrorCode end_result = peer_stop(handle);
    if (end_result.tag != Ok) {
        fprintf(stderr, "Failed to stop peer. Error code: %d\n", end_result.tag);
        return EXIT_FAILURE;
    }

    uint32_t out_counter = 0;
    ErrorCode result = peer_get_counter(handle, &out_counter);
    if (result.tag != Ok) {
        fprintf(stderr, "Failed to get counter. Error code: %d\n", result.tag);
    } else {
        printf("Current counter value: %u\n", out_counter);
    }


    printf("\n=== All FFI operations completed successfully! ===\n");
    return 0;
}
