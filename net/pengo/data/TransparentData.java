package net.pengo.data;


import java.io.*;

class TransparentData extends Data {
    //FIXME: should listen for updates from its source data (rawdata)
    protected Data sourceData;
    protected long start;
    protected long length;
    protected String name = null;
    
    public TransparentData(Data sourceData, long start, long length) {
        this.sourceData = sourceData;
        this.start = start;
        this.length = length;
        //FIXME: validate data?
    }

    //FIXME: throw out of bounds thing
    public Data getSelection(long start, long length) {
        if (start < this.start || start+length > this.start+this.length) {
            throw new IllegalArgumentException("out of bounds in transparent data");
        }
        return new TransparentData(sourceData, start, length);
    }
    

    public TransparentData subdata(long start, long length) {
        return new TransparentData(sourceData, start, length);
    }

    /** shift is how far to shift the view of the source data. used for deletes */
    public Data slideSource(long start, long length, long shiftSource) {
        // FIXME: check bounds? in super method too..
        return new ShiftedData(start, length, shiftSource, sourceData);
    }
    
    public Data shiftStart(long start, long length, long shiftStart) {
        // FIXME: check bounds? in super method too..
        return new ShiftedData(sourceData, start, length, shiftStart);
    }
    
    public Data shiftStart(long shiftStart) {
        return new ShiftedData(sourceData, getStart(), getLength(), shiftStart);
    }
    
    
    public long getStart() {
        return start;
    }
    
    public long getLength() {
        return length;
    }

    // xxx check root source somehow?
    // xxx
    public boolean equals(TransparentData rds) {
	if (rds == null)
	    return false;

        if (rds.start == this.start
                && rds.length == this.length
                && rds.sourceData == this.sourceData //FIXME: may be the same "root" source
                ) {
            return true;
        }
        
        return false;
    }
    
    public String toString() {
	//FIXME: change to (optional) hex offset display?
	if (name == null)
	    return getShortDesc();

	return name + " " + getShortDesc();
    }

    public String getShortDesc() {
        if (length == 1) {
            return "[" + start + "] source: "+ sourceData;
        } else {
            return "[" + start + "-" + (start+length-1) + "] source: "+ sourceData;
        }
    }

    public void setName(String name) {
	this.name = name;
    }

    public String getName() {
	// see also: toString()
	return name;
    }
    
    public String getType() {
        return "selection";
    }

    
    public InputStream dataStream() throws IOException {
        return sourceData.getDataStream(start, length);
    }
    
    
    public InputStream getDataStream(long offset) throws IOException {
        return sourceData.getDataStream(start+offset, length-offset);
    }
    
    // offset is relative to the board, not object's start.
    public InputStream getDataStream(long start, long len) throws IOException {
        if (start+len > this.start+length) {
            throw new IllegalArgumentException("End of requested stream is after end of object\nthis:" + this + " sourceData:" + sourceData + " request:" + start + "-" + (start+len));
        }
        if (start < this.start) {
            throw new IllegalArgumentException("Start of requested stream is before start of object\n" + this + " sourceData:" + sourceData + " request:" + start + "-" + (start+len));
        }
        return sourceData.getDataStream(start, len);
    }
    
    
    
}
