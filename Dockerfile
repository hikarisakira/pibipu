FROM oven/bun:slim

COPY ./ ./

RUN bun i
CMD [ "bun", "start" ]
