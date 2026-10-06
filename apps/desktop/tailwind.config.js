/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        tovi: {
          blue:  "#3B82F6",
          dark:  "#0F172A",
          panel: "#1E293B",
          border:"#334155",
        }
      }
    },
  },
  plugins: [],
};
