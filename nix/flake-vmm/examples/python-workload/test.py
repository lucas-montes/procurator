import time

def main():
    print("Hello, World!")
    with open("output.txt", "+x") as file:
        file.write("This is a test file.\n")

    for i in range(5):
        print(f"Count: {i}")
        time.sleep(i)

    with open("output.txt", "r") as file:
        result = file.read()
        print(f"this is the file content: {result}")

if __name__ == "__main__":
    main()
