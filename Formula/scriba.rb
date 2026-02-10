class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.16.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.16.1/scriba-x86_64-apple-darwin"
    sha256 "910c78adf3d43c7945f6abe32f9a22cfcf48a2cf800ece7854ca3d978ed0fe11"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.16.1/scriba-aarch64-apple-darwin"
    sha256 "7a2ff4e87ca3bfae7a684817aca9dac589bd648b30c232899d084ea7888e28b3"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
