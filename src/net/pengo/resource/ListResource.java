/*
 * Created on Dec 30, 2003
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.resource;

import net.pengo.dependency.QNode;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.MethodQFunction;
import net.pengo.pointer.SmartPointer;

/**
 * @author Smiley
 *
 * perhaps better named ArrayResource. This should be used a lot more.
 * 
 */
public class ListResource extends DefinitionResource {
    
    public final SmartPointer length = new JavaPointer("net.pengo.resource.IntResource");
    public final MethodQFunction elementAt = new MethodQFunction();
    public final MethodQFunction setElement = new MethodQFunction();
    
    private QNodeResource[] array;
    
    /**
     * @param openFile
     */
    public ListResource(IntResource length) {

		this.length.setName("Length");
		this.length.addSink(this);
		this.length.setValue(length);
		
		//FIXME: array length limited to int sizes
		array = new QNodeResource[length.getValue().intValue()];

		elementAt.setName("element at");
		elementAt.addSink(this);
		
		setElement.setName("set element");
		setElement.addSink(this);
		
		try {
		    elementAt.setValue(getClass().getMethod("elementAt",new Class[]{IntResource.class}));
		    setElement.setValue(getClass().getMethod("setElement",new Class[]{IntResource.class, QNodeResource.class}));
		} catch (NoSuchMethodException e) {
            //FIXME: in theory, should be caught happen compile time
            // TODO: handle exception
            e.printStackTrace();
        }
        
    }
    
    public QNodeResource elementAt(IntResource index) {
        return array[index.getValue().intValue()];
    }

    public void setElement(IntResource index, QNodeResource value) {
        array[index.getValue().intValue()] = value;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.DefinitionResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub

    }

    /* (non-Javadoc)
     * @see net.pengo.dependency.QNode#getSources()
     */
    public QNode[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }

}
