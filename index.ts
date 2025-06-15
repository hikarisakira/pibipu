import { Client, GatewayIntentBits } from "discord.js";
import { CommandKit } from "commandkit";
import mongoose from "mongoose";
import path from "path";

//創建新客戶端實例
const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent, // 暫時註解掉，需要在 Discord Developer Portal 啟用
  ],
});
// Commandkit 初始化, 載入Slash指令和事件
new CommandKit({
  client,
  commandsPath: path.join(import.meta.dir, "commands"),
  eventsPath: path.join(import.meta.dir, "events"),
  bulkRegister: true,
});
//連結至mongoDB(預設為atlas)
async function connectDB() {
  try {
    const mongoUri =
      process.env.MONGODB_URI || "mongodb://localhost:27017/mydatabase";

    // 針對 Bun 的 MongoDB 連接選項，避免 TLS 解構賦值問題
    const options = {
      serverSelectionTimeoutMS: 10000,
      socketTimeoutMS: 45000,
      maxPoolSize: 10,
      minPoolSize: 0,
      maxIdleTimeMS: 30000,
      // 禁用 TLS 相關選項以避免 Bun 的解構賦值問題
      ssl: false,
      authSource: "admin",
    };

    // 如果是 Atlas 連接，使用不同的策略
    if (mongoUri.includes("mongodb+srv://")) {
      // 對於 Atlas，使用最簡化的選項
      await mongoose.connect(mongoUri, {
        serverSelectionTimeoutMS: 10000,
      });
    } else {
      await mongoose.connect(mongoUri, options);
    }

    console.log("Connected to the database successfully.");
  } catch (error) {
    console.error("Failed to connect to the database:", error);
    console.log("嘗試使用本地 MongoDB 連接...");

    // 嘗試本地連接作為備用
    try {
      await mongoose.connect("mongodb://localhost:27017/pibipu", {
        serverSelectionTimeoutMS: 5000,
      });
      console.log("Connected to local MongoDB successfully.");
    } catch (localError) {
      console.error("本地 MongoDB 連接也失敗:", localError);
      console.log("將在沒有資料庫的情況下繼續運行...");
    }
  }
}
//機器人上線後事件
client.once("ready", async () => {
  console.log(`Logged in as ${client.user?.tag}`);
  await connectDB();
});

//處理未捕獲錯誤和未處理的拒絕
process.on("unhandledRejection", (reason, promise) => {
  console.error("Unhandled Rejection at:", promise, "reason:", reason);
});

process.on("uncaughtException", (error) => {
  console.error("Uncaught Exception:", error);
});

//登錄機器人
client.login(process.env?.DISCORD_TOKEN);
