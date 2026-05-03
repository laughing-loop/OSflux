# Write Engine Design

## Objective
Maximize throughput while maintaining correctness and stability.

---

## Key Concepts

### 1. Pipeline Architecture

Producer → Queue → Workers → Device

---

### 2. Buffering Strategy

- Default: 1MB–4MB blocks
- Adaptive resizing
- Pre-allocated buffers

---

### 3. Queue System

- Bounded queue (prevents memory explosion)
- Lock-free preferred (crossbeam)

---

### 4. Overlapped I/O (Windows)

- Multiple writes in flight
- Non-blocking execution
- Completion-based synchronization

---

### 5. Adaptive Tuning

Metrics:
- Throughput (MB/s)
- Latency
- Queue depth utilization

Adjust:
- Buffer size
- Worker count

---

## Modes

### Raw Mode
- Direct ISO → disk
- Fastest

### Filesystem Mode
- Extract + write
- Required for Windows

---

## Optimization Goals

- Minimize syscalls
- Avoid small writes
- Keep queue saturated