.PHONY: setup-tests setup-test clean-test-configs

setup-tests:
	./scripts/setup-test-projects.sh

setup-test:
	@if [ -z "$(PROJECT)" ]; then \
		echo "usage: make setup-test PROJECT=tests/Your_Project"; \
		exit 1; \
	fi
	./scripts/setup-test-projects.sh "$(PROJECT)"

clean-test-configs:
	./scripts/setup-test-projects.sh --clear-all
