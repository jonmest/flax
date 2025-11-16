# Flax - a Rust load balancer based on io_uring
Flax is an educational project. I am trying to build a level 7 load balancer based on io_uring primitives and Rust. No runtimes such as tokio-uring are used. Not being a network programmer myself, it has been a struggle making it perform as well as I would like. Going off of benchmarks on my local machine (16 cores, 64 GB RAM) the load balancer has about a 45% lower Requests-Per-Second (RPS) when compared to going straight to a Nginx backend for GET operations of small text files - I am yet to test more realistic scenarios with more IO and latency.

At the moment I am taking a break from the network related tasks and looking into implementing the SWIM protocol to make it a distributed load balancer.
