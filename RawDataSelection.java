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
        return "[" + start + "-" + (start+length) + "]";
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
