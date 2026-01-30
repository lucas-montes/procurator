
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


    printf("Dummy executable finished\n");
    return 0;
}
