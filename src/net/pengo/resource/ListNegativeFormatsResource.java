/*
 * ListNegativeFormatsResource.java
 *
 * Created on 17 September 2004, 09:52
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;

/**
 *
 * @author  Que
 */
public class ListNegativeFormatsResource extends ListSingleChoiceResource {
   
    
    /** Creates a new instance of ListNegativeFormatsResource */
    public ListNegativeFormatsResource(IntResource r) {
	super(r, getNegList());
    }
    
    //fixme: make these SmartPointers too ?
    static public SmartPointer negListP = new JavaPointer(ListPrimativeResource.class);
    static private TypeResource stringType = new JavaType(StringResource.class);

    static {
	ListPrimativeResource negList = new ListPrimativeResource(stringType);
	String[] negStrings = new String[] {"Unsigned","One's complement","Two's Completement","Sign-and-magnitude","Reserved Sign Bit (NYI)"};
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
