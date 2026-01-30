#include <stdio.h>
#include <stdlib.h>

// Declare the Rust functions
extern int add_numbers(int a, int b);
extern char* greet(const char* name);
extern void free_rust_string(char* s);
extern unsigned long long factorial(unsigned int n);

int main(int argc, char *argv[]) {
    printf("=== Rust FFI Example ===\n\n");

    // Test add_numbers
    int result = add_numbers(42, 58);
    printf("42 + 58 = %d\n", result);

    // Test greet
    char* greeting = greet("C Developer");
    if (greeting != NULL) {
        printf("%s\n", greeting);
        free_rust_string(greeting);
    }

    // Test factorial
    unsigned long long fact = factorial(10);
    printf("10! = %llu\n", fact);

    printf("\nAll FFI calls completed successfully!\n");
    return 0;
}
