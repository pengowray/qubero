import java.io.*;
import javax.swing.tree.*;

/* recursive defintion (naming/mapping) of some binary stuff */
/* when making definitions, attempt to keep useful sequences together. e.g. if a parent node has two overlapping sets of sub-data, put each set in its own non-leaf node.*/

interface Chunk {

  // what sort of thing is this called within its spec?
  public String getType();
  
  // the name for the side menu
  //public String toString();
  
  public DefaultDefinition getDef();
  
  // what sort of thing is this? (for working out the icon?)
  //public int getTypeID();
  
  /** returns the all chunk(s) within this one*/
  //public Chunk[] pos();

  /** returns the chunk(s) at this position within this chunk */
  //public Chunk[] pos(int start); // will throw "OutOfBounds" exception

  /** returns the chunk(s) between these positions within this chunk */
  //public Chunk[] pos(int start, int length); // will throw "OutOfBounds" exception

  /** launch an editor for this entire chunk. */
  //public void edit(); // throw EditorNotFound except in future 

  /** can this entire chunk be edited / is there an editor for it? */
  //public boolean isEditable(); // XXX: make this an interface?
  
  //XXX: replace with iterators and stuff
  //public byte[] getData();
}

/** 
 * RawData is a fixed length chunk of data. e.g. a binary file, or an area of memory.
 */
abstract class RawData implements Chunk {
    abstract public byte[] getData();
    abstract public int getLength();
    
    //XXX: throw out of bounds thing
    public RawDataSelection getSelection(int start, int length) {
        return new RawDataSelection(this, start, length);
    }
    
    //public Chunk getMultpleSelection(...);    
    
    public String toString() {
        return "raw data";
    }
        
    public String getType() {
        return "Raw data";
    }

    public DefaultDefinition getDef() {
        return new DefaultDefinition(this);
    }

}

class RawDataSelection extends RawData {
    //XXX: should listen for updates from its source data (rawdata)
    private RawData rawdata; //XXX: rename to sourceData
    private int start;
    private int length;
    private byte data[] = null;
    
    public int getStart() {
        return start;
    }
    
    public int getLength() {
        return length;
    }

    /*
    public boolean equals(Object o){
	if (o == null) { 
	    return false;
	} else {
	    return super.equals(o);
	}
    }
    */

    public boolean equals(RawDataSelection rds) {
	if (rds == null) 
	    return false;

        if (rds.start == this.start &&
                rds.length == this.length &&
                rds.rawdata == this.rawdata) {
            return true;
        }
        
        return false;    
    }
    
    public RawDataSelection(RawData rawdata, int start, int length) {
        this.rawdata = rawdata;
        this.start = start;
        this.length = length;
        //XXX: validate data?
    }
    
    public String toString() {
        //return "Selection of \"" + rawdata + "\" [" + start + "-" + (start+length) + "] ";
        return "Selection [" + start + "-" + (start+length) + "]";
    }
    
    public String getType() {
        return "selection";
    }
    
    public byte[] getData() {
        if (data == null) {
            data = new byte[length];
            System.arraycopy(rawdata.getData(), start, data, 0, length);
        }
        return data;
    }
    
}

/** 
 * rudimentary file editing. no extending file size.
 */
class SimpleFileChunk extends RawData {
    protected String filename;
    protected byte[] data = null;
    
    public SimpleFileChunk(String filename) {
        this.filename = filename;
        readFileToMemory();
    }

    public byte[] getData() {
        return data; // make copy?
    }

    /**
    * reads the entire (hex) file to memory for quick access
    */
    public void readFileToMemory() {
        //XXX: very inefficent! fix it sometime.
        try {
            File in = new File(filename);
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
            data = "error reading file.".getBytes();
        }
        
    }

    // make long eventually?
    public int getLength() {
        return data.length;
    }

    public String toString() {
        return filename;
    }

    public String getType() {
        return "file";
    }
    
}

// XXX: make this an inner class of RawData?
class DefaultDefinition extends DefaultMutableTreeNode implements Chunk {
    
    public DefaultDefinition (RawData rawdata) {
        super(rawdata, true);
    }
    
    public String toString() {
        return "tree yeah";
    }
    
    public String getType() {
        return "tree";
    }
    
    public DefaultDefinition getDef() {
        //XXX: is this right?
        return this;
    }
}

class RecordSetChunk {
  
}

class ByteChunk {

}

class FlagsChunk {

}

class NarrativeSequenceChunk {

}

class OverviewDisplayChunk {

}

