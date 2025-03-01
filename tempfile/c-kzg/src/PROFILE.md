# Profiling

We use [`gperftools`](https://github.com/gperftools/gperftools) (Google
Performance Tools) for profiling. Note, we also considered using
[`llvm-xray`](https://llvm.org/docs/XRay.html) but found it lacking in
comparison. This will not tell you how long (wall clock time) each function
took, but it will help you determine which functions are the most expensive.

## Prerequisites

On Linux (Debian), you need to install:
```
sudo apt install gperftools graphviz
```

On macOS, you need to install (via [homebrew](https://brew.sh)):
```
brew install gperftools ghostscript graphviz
```

## How to run

### Generating profiling graphs

There is a Makefile rule that should just auto-magically work:
```
make profile
```

For each profiled function, this will produce two files (a PROF and PDF
file). The PROF file is the raw profiling data and the PDF is the
human-friendly graph that generated from that profiling data.

#### Errors on macOS

Note, on macOS there may be a lot of "errors" like:
```
otool-classic: can't open file: /usr/lib/libc++.1.dylib
```

In my experience, you can ignore these. It's somewhat a known issue and may be
resolved later. The PDFs should still generate successfully. I think it's the
reason some function names are a hexadecimal address though.

### Viewing profiling graphs

On Linux, you can open an individual PDF file like:
```
xdg-open blob_to_kzg_commitment.pdf
```

On macOS, you can open an individual PDF file like:
```
open blob_to_kzg_commitment.pdf
```

Or, you can open all the PDF files like:
```
open *.pdf
```

### Interpreting the profiling graphs

These might not make much sense without guidance. From a high-level, this works
by polling the instruction pointer (what's being executed) at a specific rate
(like once every 5 nanoseconds) and tracking this information. From this, you
can infer the relative time each function uses by counting the number of samples
that are in each function. 

Given a box containing:
```
my_func 189 (0.6%) of 28758 (96.8%)
```

* Each box is a unique function.
* Bigger boxes are more expensive.
* Lines between boxes are function calls.
* 189 is the number of profiling samples in this function.
* 0.6% is the percentage of profiling samples in the functions.
* 28758 is the number of profiling samples in this function and its callees.
* 96.8% is the percentage of profiling samples in this function and its callees.
