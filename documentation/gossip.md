# Gossip / SWIM
I will implement the SWIM protocol in order to support a distributed load balancer. See https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/SWIM.pdf and in particular section 3 describing the SWIM approach.

We will have a failure detector component that detects failures of members and a dissemination component disseminating gossip about members having recently joined or left the cluster. 

Each member will have a list of N other cluster members. This list will be shuffled each time a member is added or removed. The member will maintain an index referencing a member in the shuffled list, and every period T' they will ping that member and increment the index (modulo N). This ensures that pings across the entire cluster are spread out, and that every member does not ping the same node at the same time. Shuffling the list on membership changes rather than randomly selecting a member from the list every ping also ensures that the maximum time until a node pings a particular member is bounded by T' * N. 

If a node does not receive a response when pinging the selected member within the predetermined timeout, the node "indirectly probes it" by selecting k mother members in the list, telling them that the other member is suspect and asking them to ping it on their behalf.
