FROM debian

LABEL key="georgios@paradigm.xyz"

RUN apt-get update -y; apt-get install -y curl git
WORKDIR /root
ADD . .

ENV SHELL "/bin/bash"
RUN ["/bin/bash", "foundryup/install"]

ENV PATH "$PATH:/root/.foundry/bin/"
RUN echo "export PATH=/new/path:${PATH}" >> $HOME/.bashrc; \
    foundryup
