/*
 * Created on Jan 21, 2004
 */
package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.QFunction;

/**
 * @author Smiley
 */
public class CallSignature extends Resource {
    
    // subject.name(parameters[]) throws ExceptionHanlders[] ;
    // ExceptionHanlders == QFunctions that take some sort of exception argument
    public JavaPointer funciton; // = QFunction
    public String id;
    
    // TypeResources 
    public final JavaPointer subject = new JavaPointer("net.pengo.resource.TypeResource");
    public final ListResource parameters = new ListPrimativeResource("net.pengo.resource.TypeResource");
    public final JavaPointer returnValue = new JavaPointer("net.pengo.resource.TypeResource");
    
    // these must be types compatible with QFunction's Type 
    public final JavaPointer exceptions = new JavaPointer("net.pengo.resource.ListResource");;
    public final JavaPointer continuation = new JavaPointer("net.pengo.resource.TypeResource");; 
    
    //FIXME: make "object" part of param ('owner' parameter)
    //FIXME: make it: public void invoke(SmartPointer result, ListResource param){
    //FIXME: make "result" part of param too ('writable' param)
    //FIXME: add an exceptions param too 
    //FIXME: add access bits (public/protected/etc)
    //FIXME: add continuation as a param    
    
    public CallSignature(TypeResource subj, TypeResource[] params) {
        super();
        subject.setName("Subject type");
        subject.addSink(this);
        subject.setValue(subj);
        
        parameters.setName("Parameter types");
        parameters.addSink(this);
        parameters.setValue(params);
        
        
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#getSources()
     */
    public Resource[] getSources() {
        return new Resource[] {};
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#canConvertTo(net.pengo.resource.TypeResource, net.pengo.resource.Aspect)
     */
    public boolean canConvertTo(TypeResource type, Aspect aspect) {
        // TODO Auto-generated method stub
        return false;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#canConvertTo(net.pengo.resource.TypeResource)
     */
    public boolean canConvertTo(TypeResource x) {
        // TODO Auto-generated method stub
        return false;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#canIsomorphicConvert(net.pengo.resource.TypeResource, net.pengo.resource.Aspect)
     */
    public boolean canIsomorphicConvert(TypeResource x, Aspect aspect) {
        // TODO Auto-generated method stub
        return false;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#getFunctions()
     */
    public QFunction[] getFunctions() {
        // TODO Auto-generated method stub
        return null;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#getConstructors()
     */
    public QFunction[] getConstructors() {
        // TODO Auto-generated method stub
        return null;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#equals(net.pengo.resource.TypeResource)
     */
    public boolean equals(TypeResource tr) {
        // TODO Auto-generated method stub
        return false;
    }
    
    //public 

}
