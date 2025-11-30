#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysinfo.h>

// s
void print_memory_info() {
    FILE *file = fopen("/proc/meminfo", "r");
    if (file == NULL) {
        printf("Error opening /proc/meminfo\n");
        return;
    }

    char line[256];
    unsigned long mem_total = 0, mem_free = 0, mem_available = 0;

    while (fgets(line, sizeof(line), file)) {
        if (sscanf(line, "MemTotal: %lu kB", &mem_total) == 1) continue;
        if (sscanf(line, "MemFree: %lu kB", &mem_free) == 1) continue;
        if (sscanf(line, "MemAvailable: %lu kB", &mem_available) == 1) continue;
    }
    fclose(file);

    printf("Memory - Total: %lu MB, Free: %lu MB, Available: %lu MB\n",
           mem_total / 1024, mem_free / 1024, mem_available / 1024);
}

void print_cpu_info() {
    FILE *file = fopen("/proc/stat", "r");
    if (file == NULL) {
        printf("Error opening /proc/stat\n");
        return;
    }

    char line[256];
    unsigned long user, nice, system, idle, iowait, irq, softirq, steal;

    if (fgets(line, sizeof(line), file)) {
        sscanf(line, "cpu %lu %lu %lu %lu %lu %lu %lu %lu",
               &user, &nice, &system, &idle, &iowait, &irq, &softirq, &steal);

        unsigned long total = user + nice + system + idle + iowait + irq + softirq + steal;
        unsigned long used = total - idle - iowait;

        printf("CPU - Used: %lu, Idle: %lu, Total: %lu (Usage: %.2f%%)\n",
               used, idle + iowait, total, (double)used / total * 100.0);
    }
    fclose(file);

    printf("CPU Cores: %d\n", get_nprocs());
}

void print_process_limits() {
    FILE *file = fopen("/proc/self/limits", "r");
    if (file == NULL) {
        printf("Error opening /proc/self/limits\n");
        return;
    }

    char line[512];
    printf("\n=== Process Resource Limits ===\n");
    while (fgets(line, sizeof(line), file)) {
        if (strstr(line, "Max memory size") ||
            strstr(line, "Max cpu time") ||
            strstr(line, "Max processes")) {
            printf("%s", line);
        }
    }
    fclose(file);
}

void print_cgroup_info() {
    // Check if running in a cgroup
    FILE *file = fopen("/proc/self/cgroup", "r");
    if (file == NULL) {
        printf("Error opening /proc/self/cgroup\n");
        return;
    }

    char line[512];
    printf("\n=== Cgroup Information ===\n");
    while (fgets(line, sizeof(line), file)) {
        printf("%s", line);
    }
    fclose(file);

    // Try to read memory limit from cgroup
    file = fopen("/sys/fs/cgroup/memory/memory.limit_in_bytes", "r");
    if (file != NULL) {
        unsigned long limit;
        if (fscanf(file, "%lu", &limit) == 1) {
            printf("Cgroup Memory Limit: %lu MB\n", limit / (1024 * 1024));
        }
        fclose(file);
    }
}

int main(int argc, char *argv[])
{
    printf("Dummy C executable (non master) started (PID: %d)\n", getpid());

    // Default behavior: sleep for 20 seconds
    int sleep_time = 20;

    // If argument provided, use it as sleep time
    if (argc > 1)
    {
        sleep_time = atoi(argv[1]);
    }

    int iteration = 0;
    for (;;)
    {
        printf("\n=== Iteration %d ===\n", ++iteration);

        // Print system resource information
        print_memory_info();
        print_cpu_info();
        print_process_limits();
        print_cgroup_info();

        printf("\nSleeping for %d seconds...\n", sleep_time);
        sleep(sleep_time);
    }

    printf("Dummy executable finished\n");
    return 0;
}
