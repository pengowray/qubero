class RawDataSelection extends RawData {
    //XXX: should listen for updates from its source data (rawdata)
    protected DefNode defnode = null; // may be left null
    protected RawData sourceData;
    protected int start;
    protected int length;
    protected byte data[] = null;
    protected String name = null;
    
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
                rds.sourceData == this.sourceData) {
            return true;
        }
        
        return false;    
    }
    
    public RawDataSelection(DefNode defnode, int start, int length) {
	this.defnode = defnode;
	this.sourceData = defnode.getRawData();
        this.start = start;
        this.length = length;
        //XXX: validate data?
    }

    public RawDataSelection(RawData sourceData, int start, int length) {
        this.sourceData = sourceData;
        this.start = start;
        this.length = length;
        //XXX: validate data?
    }
    
    public String toString() {
	//XXX: change to (optional) hex offset display?
	if (name == null)
	    return getShortDesc();

	return name + " " + getShortDesc();
    }

    public String getShortDesc() {
	return "[" + start + "-" + (start+length) + "]";
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
    
    public byte[] getData() {
        if (data == null) {
            data = new byte[length];
            System.arraycopy(sourceData.getData(), start, data, 0, length);
        }
        return data;
    }

    public DefNode getDefNode() {
	return defnode;
    }

}
