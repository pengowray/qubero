/*
 * ListEndianResource.java
 *
 * Copy of ListNegativeFormatsResource
 *
 * Created on 3 October 2004, 09:52
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;

/**
 *
 * @author  Peter Halasz
 */
public class ListEndianResource extends ListSingleChoiceResource {
    
    /** Creates a new instance of ListEndianResource */
    public ListEndianResource(IntResource r) {
	super(r, getNegList());
    }    
    
    //fixme: make these SmartPointers too ?
    static public SmartPointer negListP = new JavaPointer(ListPrimativeResource.class);
    static private TypeResource stringType = new JavaType(StringResource.class);

    static {
	ListPrimativeResource negList = new ListPrimativeResource(stringType);
	String[] negStrings = new String[] {"Network byte-order (big endian)","Little endian (Intel)"};
	negList.setCount(new IntPrimativeResource(negStrings.length));
	for (int c=0; c<negStrings.length; c++) {
	    negList.setElement(new IntPrimativeResource(c), new StringPrimativeResource(negStrings[c]));
	}
	
	negListP.setValue(negList);
    }
    
    static private ListResource getNegList() {
	return (ListPrimativeResource)negListP.evaluate();
    }
    
}
