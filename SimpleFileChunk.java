import java.io.*;

/** 
 * rudimentary file editing. no extending file size.
 */
class SimpleFileChunk extends RawData {
    protected File file;
    protected byte[] data = null;
    protected String name;

    public SimpleFileChunk(File file) {
	this.file = file;
	name = file.getName();
        readFileToMemory();
    }	

    public SimpleFileChunk(String filename) {
        this(new File(filename));
    }

    public byte[] getData() {
        return data; // make copy?
    }

    /**
    * reads the entire (hex) file to memory for quick access
    */
    public void readFileToMemory() {
        //XXX: very inefficent! fix it sometime.
	//XXX: should set off a trigger if called after construction
        try {
            File in = file;
            FileInputStream fis = new FileInputStream(in);

            // read in bytes to data

            int ch; // current byte
            int chc = 0; // char count
            if (data == null)
                data = new byte[1024];
            while ((ch=fis.read()) != -1) {
                if (data.length <= chc) {
                    // double data's size
                    byte[] dataTemp = new byte[data.length*2] ;
                    System.arraycopy(data,0,dataTemp,0,data.length);
                    data = dataTemp;
                }
                data[chc] = (byte)ch;
                chc++;
            }
            
            // truncate data to correct size
            if (data.length != chc) {
                byte[] dataTemp = new byte[chc] ;
                System.arraycopy(data,0,dataTemp,0,chc);
                data = dataTemp;
            }
            
        } catch (IOException e) {
            //FIXME: !!!
            data = new String("error reading file: " + e).getBytes();
        }
        
    }

    // make long eventually?
    public int getLength() {
        return data.length;
    }

    public String toString() {
        return name;
    }

    public String getType() {
        return "file";
    }
    
}
