.PHONY: all install test lint format check build clean dev

# Default target
all: install test lint

# Install all dependencies
install:
	pnpm install
	pip install -r requirements.txt
	pip install -e ".[dev]"

# Run all tests
test:
	pnpm test
	pytest

# Run linters
lint:
	pnpm lint
	black --check .
	ruff check .
	mypy ml_service

# Format code
format:
	pnpm format
	black .
	ruff --fix .

# Type checking
check:
	pnpm typecheck
	mypy ml_service

# Build everything
build:
	pnpm build

# Clean build artifacts and dependencies
clean:
	rm -rf node_modules
	rm -rf dist
	rm -rf build
	rm -rf *.egg-info
	rm -rf .pytest_cache
	rm -rf .mypy_cache
	rm -rf __pycache__
	find . -type d -name "__pycache__" -exec rm -rf {} +

# Development mode
dev:
	pnpm dev & uvicorn ml_service.main:app --reload

# Run Docker Compose
docker-up:
	docker-compose up -d

docker-down:
	docker-compose down

# Database migrations
migrate:
	alembic upgrade head

# Install system dependencies (Debian/Ubuntu)
install-system-deps:
	apt-get update && apt-get install -y \
		postgresql-client \
		libpq-dev \
		gcc \
		python3-dev \
		nodejs \
		npm
