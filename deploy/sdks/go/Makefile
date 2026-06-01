.PHONY: gen build test tidy lint clean

gen:
	bash ../../proto-gen/scripts/gen-go.sh

build: gen
	cd example && go build .

test:
	go test ./...

tidy:
	go mod tidy

lint:
	go vet ./...
	which golangci-lint > /dev/null && golangci-lint run ./... || true

clean:
	rm -rf efev1/ example/example
