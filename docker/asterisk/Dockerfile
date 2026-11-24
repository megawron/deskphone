FROM alpine:latest

RUN apk update && apk add --no-cache \
    asterisk \
    asterisk-sample-config \
    && rm -rf /var/cache/apk/*

COPY asterisk.conf /etc/asterisk/asterisk.conf
COPY modules.conf /etc/asterisk/modules.conf
COPY pjsip.conf /etc/asterisk/pjsip.conf
COPY extensions.conf /etc/asterisk/extensions.conf
COPY manager.conf /etc/asterisk/manager.conf
COPY rtp.conf /etc/asterisk/rtp.conf

EXPOSE 5060/udp 5060/tcp 5038/tcp 10000-10050/udp

CMD ["/usr/sbin/asterisk", "-f", "-vvv"]
