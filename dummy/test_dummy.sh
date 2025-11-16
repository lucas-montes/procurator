#!/usr/bin/env bash

# Simulate a build process with randomness
# 80% success rate, 20% failure rate

echo "Running dummy build process..."

# Generate random number between 1-100
RANDOM_NUM=$((1 + RANDOM % 100))

if [ $RANDOM_NUM -le 80 ]; then
    echo "✓ Build succeeded!"
    exit 0
else
    echo "✗ Build failed!"
    exit 1
fi
