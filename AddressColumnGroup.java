/**
 * AddressColumnGroup.java
 *
 * @author Created by Omnicore CodeGuide
 */


public class AddressColumnGroup extends AbstractColumnGroup
{
    private OpenFile openFile;
    
    public AddressColumnGroup(OpenFile openFile) {
	this.openFile = openFile;
	calcWidth();
    }
    
    private void calcWidth() {
	// calculcuates and caches width of the address column
	Data d = openFile.getData();
	long len = d.getLength();
	//xxx
    }
    
    public int getColumnCount() {
	return 1;
    }
    
    public Column getColumn(int col) {
    	
    }
    
    public int getPreferredWidth();
    public int getMaximumWidth();
    public int getMinimumWidth();
}

