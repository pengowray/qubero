/**
 * AddressColumnGroup.java
 *
 * @author Created by Omnicore CodeGuide
 */


package net.pengo.hexdraw.tabled;
import net.pengo.app.*;
import net.pengo.data.*;

//fixme: not meant to be abstract
abstract public class AddressColumnGroup extends AbstractColumnGroup
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
	//FIXME:
    }
    
    public int getColumnCount() {
	return 1;
    }
    
    //public Column getColumn(int col);
    
    abstract public int getPreferredWidth();
    abstract public int getMaximumWidth();
    abstract public int getMinimumWidth();
    
}

