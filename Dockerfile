FROM node:18-slim
WORKDIR /app
COPY package*.json ./
RUN npm install
COPY server.js .
EXPOSE 8080
EXPOSE 4070
CMD ["node", "server.js"]
